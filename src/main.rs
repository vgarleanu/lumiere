slint::include_modules!();

use glow::HasContext;
use std::os::raw::c_void;
use std::os::raw::c_char;
use std::ffi::CStr;
use std::ptr;

unsafe extern "C" fn get_proc_address(ctx: *mut c_void, proc: *const c_char) -> *mut c_void {
    let closure: *mut &dyn for<'r> Fn(&'r str) -> *const c_void = unsafe { std::mem::transmute(ctx) };
    let proc_name = CStr::from_ptr(proc).to_str().unwrap();

    println!("trying {}", proc_name);
    (*closure)(proc_name) as *mut c_void
}

struct EGLUnderlay {
    gl: glow::Context,
    vbo: glow::Buffer,
    vao: glow::VertexArray,
    start_time: instant::Instant,
    mpv: Box<mpv::MpvHandlerWithGl>,
}

impl EGLUnderlay {
    fn new(gl: glow::Context, proc_addr: *mut c_void) -> Self {
        let mpv_hndl = mpv::MpvHandlerBuilder::new().expect("Failed to build mpv handler")
            .build_with_gl(Some(get_proc_address), proc_addr)
            .unwrap();

        unsafe {
            let vbo = gl.create_buffer().expect("Cannot create buffer");
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));

            let vertices = [-1.0f32, 1.0f32, -1.0f32, -1.0f32, 1.0f32, 1.0f32, 1.0f32, -1.0f32];

            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, vertices.align_to().1, glow::STATIC_DRAW);

            let vao = gl.create_vertex_array().expect("Cannot create vertex array");
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_vertex_array(None);

            Self {
                gl,
                vbo,
                vao,
                start_time: instant::Instant::now(),
                mpv: mpv_hndl
            }
        }
    }
}

impl Drop for EGLUnderlay {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_vertex_array(self.vao);
            self.gl.delete_buffer(self.vbo);
        }
    }
}

impl EGLUnderlay {
    fn render(&mut self, rotation_enabled: bool) {
        unsafe {
            self.mpv.draw(0, 1920, 1080).unwrap();
            let gl = &self.gl;

            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));

            gl.bind_vertex_array(Some(self.vao));

            let elapsed = self.start_time.elapsed().as_millis() as f32;

            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);

            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_vertex_array(None);
            gl.use_program(None);
        }
    }
}

pub fn main() {
    let app = App::new();

    let mut underlay = None;

    let app_weak = app.as_weak();

    if let Err(error) = app.window().set_rendering_notifier(move |state, graphics_api| {
        // eprintln!("rendering state {:#?}", state);

        match state {
            slint::RenderingState::RenderingSetup => {
                let (context, proc_addr) = match graphics_api {
                    slint::GraphicsAPI::NativeOpenGL { get_proc_address } => unsafe {
                        (glow::Context::from_loader_function(|s| get_proc_address(s)), get_proc_address)
                    },
                    _ => return,
                };

                let ctx = Box::into_raw(Box::new(*proc_addr));

                underlay = Some(EGLUnderlay::new(context, ctx as *mut c_void))
            }
            slint::RenderingState::BeforeRendering => {
                if let (Some(underlay), Some(app)) = (underlay.as_mut(), app_weak.upgrade()) {
                    underlay.render(app.get_rotation_enabled());
                    app.window().request_redraw();
                }
            }
            slint::RenderingState::AfterRendering => {}
            slint::RenderingState::RenderingTeardown => {
                drop(underlay.take());
            }
            _ => {}
        }
    }) {
        match error {
            slint::SetRenderingNotifierError::Unsupported => eprintln!("This example requires the use of the GL backend. Please run with the environment variable SLINT_BACKEND=GL set."),
            _ => unreachable!()
        }
        std::process::exit(1);
    }

    app.run();
}
