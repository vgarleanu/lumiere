pub mod video;

slint::include_modules!();

use std::os::raw::c_void;
use video::VideoUnderlay;

pub fn main() {
    let mut args = std::env::args();
    args.next();

    let file = args.next().unwrap();
    let app = App::new();

    let mut underlay = None;
    let mut prev_is_paused = false;

    let app_weak = app.as_weak();

    if let Err(error) = app
        .window()
        .set_rendering_notifier(move |state, graphics_api| {
            let wh = if let Some(app) = app_weak.upgrade() {
                let sf = app.window().scale_factor();
                let phys_width = app.get_window_width() * sf;
                let phys_height = app.get_window_height() * sf;
                (phys_width, phys_height)
            } else {
                (2000.0, 2000.0)
            };

            match state {
                slint::RenderingState::RenderingSetup => {
                    let (context, proc_addr) = match graphics_api {
                        slint::GraphicsAPI::NativeOpenGL { get_proc_address } => unsafe {
                            (
                                glow::Context::from_loader_function(|s| get_proc_address(s)),
                                get_proc_address,
                            )
                        },
                        _ => return,
                    };

                    underlay = Some(VideoUnderlay::new(
                        context,
                        proc_addr as *const _ as *mut c_void,
                        &file,
                        wh,
                    ))
                }
                slint::RenderingState::BeforeRendering => {
                    if let (Some(underlay), Some(app)) = (underlay.as_mut(), app_weak.upgrade()) {
                        let is_paused = app.get_is_paused();
                        if prev_is_paused != is_paused {
                            if is_paused {
                                underlay.pause();
                            } else {
                                underlay.play();
                            }

                            prev_is_paused = is_paused;
                        }

                        if !app.get_position_ackd() {
                            app.set_position_ackd(true);
                            let duration = underlay.get_duration().unwrap_or(0) as f64;
                            let seek_target = app.get_new_position() as f64 / 100.0 * duration;
                            underlay.get_mpv().seek_absolute(seek_target);
                        }

                        app.set_ts_label(underlay.get_ts_label().into());

                        let position = underlay.get_position().unwrap_or(0) as f32;
                        let duration = underlay.get_duration().unwrap_or(0) as f32;

                        app.set_seek_position(position / duration);
                        underlay.render(wh);
                        app.window().request_redraw();
                    }
                }
                slint::RenderingState::AfterRendering => {}
                slint::RenderingState::RenderingTeardown => {
                    drop(underlay.take());
                }
                _ => {}
            }
        })
    {
        match error {
            slint::SetRenderingNotifierError::Unsupported => eprintln!("This example requires the use of the GL backend. Please run with the environment variable SLINT_BACKEND=GL set."),
            _ => unreachable!()
        }
        std::process::exit(1);
    }

    app.run();
}
