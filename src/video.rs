use glow::HasContext;

use std::mem::transmute;
use std::os::raw::c_void;
use std::time::Duration;

use libmpv::render::*;
use libmpv::*;

fn get_proc_address(ctx: &*mut c_void, proc: &str) -> *mut c_void {
    unsafe { (*((*ctx) as *const &dyn Fn(&str) -> *mut c_void))(proc) as *mut c_void }
}

pub struct VideoUnderlay {
    gl: glow::Context,
    fbo: glow::Framebuffer,
    texture: glow::Texture,
    depth_texture: glow::Texture,
    start_time: instant::Instant,
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::NativeBuffer,
    prev_width: f32,
    prev_height: f32,

    // NOTE: The order of the fields below should not change otherwise we will get a SEGFAULT.
    render_ctx: RenderContext,
    mpv: Mpv,
}

impl VideoUnderlay {
    pub fn new(gl: glow::Context, proc_addr_ctx: *mut c_void, file: &str, wh: (f32, f32)) -> Self {
        let mut mpv = Mpv::new().expect("Error while creating MPV");
        let render_ctx = RenderContext::new(
            unsafe { mpv.ctx.as_mut() },
            vec![
                RenderParam::ApiType(RenderParamApiType::OpenGl),
                RenderParam::InitParams(OpenGLInitParams {
                    get_proc_address,
                    ctx: proc_addr_ctx,
                }),
            ],
        )
        .expect("Failed creating render context");

        mpv.event_context_mut().disable_deprecated_events().unwrap();
        mpv.playlist_load_files(&[(file, FileState::AppendPlay, None)])
            .unwrap();

        Self::init_gl(gl, wh, mpv, render_ctx)
    }

    fn init_gl(
        gl: glow::Context,
        (width, height): (f32, f32),
        mpv: Mpv,
        render_ctx: RenderContext,
    ) -> Self {
        unsafe {
            let program = gl.create_program().expect("Cannot create program");

            let shader_sources = [
                (glow::VERTEX_SHADER, include_str!("./vertex.glsl")),
                (glow::FRAGMENT_SHADER, include_str!("./fragment.glsl")),
            ];

            let mut shaders = Vec::with_capacity(shader_sources.len());

            for (shader_type, shader_source) in shader_sources.iter() {
                let shader = gl
                    .create_shader(*shader_type)
                    .expect("Cannot create shader");
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
                -1.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 1.0, -1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 1.0,
                1.0, -1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0,
            ];

            let vbo = gl.create_buffer().expect("Cannot create buffer");
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));

            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                quad_verts.align_to().1,
                glow::STATIC_DRAW,
            );

            let vao = gl
                .create_vertex_array()
                .expect("Cannot create vertex array");
            gl.bind_vertex_array(Some(vao));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 16, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, 16, 8);

            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_vertex_array(None);

            let fbo = gl.create_framebuffer().unwrap();
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));

            let texture = gl.create_texture().unwrap();
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGB as i32,
                width as _,
                height as _,
                0,
                glow::RGB,
                glow::UNSIGNED_BYTE,
                None,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );

            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );

            let depth_texture = gl.create_texture().unwrap();
            gl.bind_texture(glow::TEXTURE_2D, Some(depth_texture));
            gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::DEPTH24_STENCIL8, 5000, 5000);
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::DEPTH_STENCIL_ATTACHMENT,
                glow::TEXTURE_2D,
                Some(depth_texture),
                0,
            );

            assert_eq!(
                gl.check_framebuffer_status(glow::FRAMEBUFFER),
                glow::FRAMEBUFFER_COMPLETE
            );

            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);

            Self {
                gl,
                start_time: instant::Instant::now(),
                fbo,
                texture,
                depth_texture,
                program,
                vao,
                vbo,
                prev_width: width,
                prev_height: height,
                mpv,
                render_ctx,
            }
        }
    }

    pub fn pause(&self) {
        self.mpv.pause();
    }

    pub fn play(&self) {
        self.mpv.unpause();
    }

    pub fn get_position(&self) -> Option<i64> {
        self.mpv.get_property::<i64>("time-pos").ok()
    }

    pub fn get_duration(&self) -> Option<i64> {
        self.mpv.get_property::<i64>("duration").ok()
    }

    pub fn get_ts_label(&self) -> String {
        fn secs_to_pretty(secs: i64) -> String {
            let seconds = secs % 60;
            let minutes = (secs / 60) % 60;
            let hours = (secs / 60) / 60;

            format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
        }

        let position = self.get_position().unwrap_or(0);
        let duration = self.get_duration().unwrap_or(0);

        format!(
            "{} / {}",
            secs_to_pretty(position),
            secs_to_pretty(duration)
        )
    }

    pub fn get_mpv(&mut self) -> &mut Mpv {
        &mut self.mpv
    }

    pub fn render(&mut self, (width, height): (f32, f32)) {
        unsafe {
            let gl = &self.gl;

            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.clear_color(0.1, 0.1, 0.1, 1.0);

            if width != self.prev_width || height != self.prev_height {
                gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGB as i32,
                    width as _,
                    height as _,
                    0,
                    glow::RGB,
                    glow::UNSIGNED_BYTE,
                    None,
                );
                gl.bind_texture(glow::TEXTURE_2D, None);

                self.prev_width = width;
                self.prev_height = height;
            }

            self.render_ctx
                .render::<*mut c_void>(transmute(self.fbo), width as _, height as _, true)
                .expect("Failed to render");

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

impl Drop for VideoUnderlay {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_program(self.program);
            self.gl.delete_buffer(self.vbo);
            self.gl.delete_vertex_array(self.vao);
            self.gl.delete_texture(self.texture);
            self.gl.delete_texture(self.depth_texture);
            self.gl.delete_framebuffer(self.fbo);
        }
    }
}
