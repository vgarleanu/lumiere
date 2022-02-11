slint::include_modules!();

use glow::HasContext;
use std::os::raw::c_void;
use std::os::raw::c_char;
use std::ffi::CStr;
use std::ffi::CString;
use std::ptr;
use std::mem::transmute;

use libmpv_sys::*;

unsafe extern "C" fn get_proc_address(ctx: *mut c_void, proc: *const c_char) -> *mut c_void {
    let proc_name = CStr::from_ptr(proc).to_str().unwrap();

    (*(ctx as *const &dyn Fn(&str) -> *mut c_void))(proc_name) as *mut c_void
}

struct EGLUnderlay {
    gl: glow::Context,
    fbo: glow::Framebuffer,
    texture: glow::Texture,
    start_time: instant::Instant,
    mpv_handle: *mut mpv_handle,
    mpv_gl: *mut mpv_render_context,
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::NativeBuffer,
}

impl EGLUnderlay {
    fn new(gl: glow::Context, proc_addr_ctx: *mut c_void, file: &str) -> Self {
        unsafe {
            let mpv_handle = mpv_create();
            assert!(mpv_initialize(mpv_handle) >= 0);
            let log_level = CString::new("debug").unwrap();
            mpv_request_log_messages(mpv_handle, log_level.as_ptr());

            let mut params = [
                mpv_render_param {
                    type_: mpv_render_param_type_MPV_RENDER_PARAM_API_TYPE,
                    data: MPV_RENDER_API_TYPE_OPENGL as *const _ as *mut c_void,
                },
                mpv_render_param {
                    type_: mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                    data: &mut mpv_opengl_init_params {
                        get_proc_address: Some(get_proc_address),
                        get_proc_address_ctx: proc_addr_ctx,
                        extra_exts: ptr::null(),
                    } as *mut _ as *mut c_void,
                },
                mpv_render_param {
                    type_: mpv_render_param_type_MPV_RENDER_PARAM_ADVANCED_CONTROL,
                    data: &mut 0 as *mut i32 as *mut c_void,
                },
                mpv_render_param {
                    type_: 0,
                    data: ptr::null_mut(),
                }
            ];

            let mut mpv_gl: *mut mpv_render_context = ptr::null_mut();
            let result = mpv_render_context_create(&mut mpv_gl as *mut _, mpv_handle, params.as_mut_slice().as_mut_ptr());
            assert!(result >= 0);

            let command_type = CString::new("loadfile").unwrap();
            let file = CString::new(file).unwrap();
            let mut command = [
                command_type.as_ptr(),
                file.as_ptr(),
                ptr::null(),
            ];

            let result = mpv_command(mpv_handle, command.as_mut().as_mut_ptr());

            let program = gl.create_program().expect("Cannot create program");

            let shader_sources = [
                (glow::VERTEX_SHADER, include_str!("./vertex.glsl")),
                (glow::FRAGMENT_SHADER, include_str!("./fragment.glsl")),
            ];

            let mut shaders = Vec::with_capacity(shader_sources.len());

            for (shader_type, shader_source) in shader_sources.iter() {
                let shader = gl.create_shader(*shader_type).expect("Cannot create shader");
                gl.shader_source(shader, shader_source);
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    panic!("{}", gl.get_shader_info_log(shader));
                }
                gl.attach_shader(program, shader);
                shaders.push(shader);
            }

            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                panic!("{}", gl.get_program_info_log(program));
            }

            for shader in shaders {
                gl.detach_shader(program, shader);
                gl.delete_shader(shader);
            }

            let quad_verts: [f32; 24] = [
                // positions   // texCoords
                -1.0,  1.0,    0.0, 1.0,
                -1.0, -1.0,    0.0, 0.0,
                1.0,  -1.0,    1.0, 0.0,
                -1.0,  1.0,    0.0, 1.0,
                1.0,  -1.0,    1.0, 0.0,
                1.0,   1.0,    1.0, 1.0
            ];

            let quad_vao = gl.create_vertex_array().unwrap();
            let quad_vbo = gl.create_buffer().unwrap();

            let vbo = gl.create_buffer().expect("Cannot create buffer");
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));

            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, quad_verts.align_to().1, glow::STATIC_DRAW);

            let vao = gl.create_vertex_array().expect("Cannot create vertex array");
            gl.bind_vertex_array(Some(vao));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 16, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, 16, 8);

            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_vertex_array(None);

            /*
            let screen_texture_location = gl.get_uniform_location(program, "screenTexture").unwrap();
            gl.uniform_1_i32(Some(&screen_texture_location), 0);
            */

            let fbo = gl.create_framebuffer().unwrap();
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));

            let texture = gl.create_texture().unwrap();
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGB as i32, 1920, 1080, 0, glow::RGB, glow::UNSIGNED_BYTE, None);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);

            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(texture), 0);

            let depth_attachment = gl.create_texture().unwrap();
            gl.bind_texture(glow::TEXTURE_2D, Some(depth_attachment));
            gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::DEPTH24_STENCIL8, 1920, 1080);
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::DEPTH_STENCIL_ATTACHMENT, glow::TEXTURE_2D, Some(depth_attachment), 0);

            assert_eq!(gl.check_framebuffer_status(glow::FRAMEBUFFER), glow::FRAMEBUFFER_COMPLETE);

            gl.bind_framebuffer(glow::FRAMEBUFFER, None);

            Self {
                gl,
                start_time: instant::Instant::now(),
                mpv_handle,
                mpv_gl,
                fbo,
                texture,
                program,
                vao,
                vbo,
            }
        }
    }
}

impl Drop for EGLUnderlay {
    fn drop(&mut self) {
        unsafe {
        }
    }
}

impl EGLUnderlay {
    fn render(&mut self, rotation_enabled: bool) {
        unsafe {
            let gl = &self.gl;

            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.clear_color(0.1, 0.1, 0.1, 1.0);

            let mut params = [
                mpv_render_param {
                    type_: mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_FBO,
                    data: &mut mpv_opengl_fbo {
                        fbo: transmute(self.fbo),
                        w: 1920,
                        h: 1080,
                        internal_format: 0,
                    } as *mut _ as *mut c_void,
                },

                mpv_render_param {
                    type_: mpv_render_param_type_MPV_RENDER_PARAM_FLIP_Y,
                    data: &mut 1 as *mut i32 as *mut c_void,
                },
                mpv_render_param {
                    type_: 0,
                    data: ptr::null_mut()
                },
            ];

            mpv_render_context_render(self.mpv_gl, params.as_mut().as_mut_ptr());
            let elapsed = self.start_time.elapsed().as_millis() as f32;

            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);

            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            gl.draw_arrays(glow::TRIANGLES, 0, 6);

            gl.bind_vertex_array(None);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.use_program(None);
        }
    }
}

pub fn main() {
    let mut args = std::env::args();
    args.next();

    let file = args.next().unwrap();
    let app = App::new();

    let mut underlay = None;

    let app_weak = app.as_weak();

    if let Err(error) = app.window().set_rendering_notifier(move |state, graphics_api| {
        match state {
            slint::RenderingState::RenderingSetup => {
                let (context, proc_addr) = match graphics_api {
                    slint::GraphicsAPI::NativeOpenGL { get_proc_address } => unsafe {
                        (glow::Context::from_loader_function(|s| get_proc_address(s)), get_proc_address)
                    },
                    _ => return,
                };

                underlay = Some(EGLUnderlay::new(context, proc_addr as *const _ as *mut c_void, &file))
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
