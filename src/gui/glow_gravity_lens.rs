//! OpenGL 后处理黑洞透镜。

use eframe::egui;
use glow::HasContext as _;
use std::sync::{Arc, Mutex};

/// 哈基米黑洞透镜每帧传给 OpenGL 回调的绘制参数。
#[derive(Clone, Copy, Debug)]
pub(crate) struct GravityLensPaint {
    /// 透镜矩形在 egui 逻辑坐标里的位置。
    pub rect: egui::Rect,
    /// 透镜中心在 egui 逻辑坐标里的位置。
    pub center: egui::Pos2,
    /// 透镜影响半径，单位是 egui 逻辑点。
    pub radius: f32,
    /// 当前动画时间，单位秒。
    pub time_seconds: f32,
}

/// OpenGL 后处理所需的可复用资源。
pub(crate) struct GravityLensGlResources {
    /// OpenGL context，用于 Drop 阶段释放资源。
    gl: Arc<glow::Context>,
    /// OpenGL program，包含透镜后处理 vertex/fragment shader。
    program: glow::Program,
    /// 局部 framebuffer 拷贝纹理。
    texture: glow::Texture,
    /// 全屏矩形 VAO。
    vertex_array: glow::VertexArray,
    /// 全屏矩形顶点 buffer。
    vertex_buffer: glow::Buffer,
    /// uniform: 局部纹理尺寸。
    u_size: Option<glow::UniformLocation>,
    /// uniform: framebuffer 拷贝纹理 sampler。
    u_sampler: Option<glow::UniformLocation>,
    /// uniform: 透镜中心在局部纹理内的像素坐标。
    u_center: Option<glow::UniformLocation>,
    /// uniform: 透镜半径，单位像素。
    u_radius: Option<glow::UniformLocation>,
    /// uniform: 当前动画时间。
    u_time: Option<glow::UniformLocation>,
    /// 纹理当前宽度。
    texture_width: i32,
    /// 纹理当前高度。
    texture_height: i32,
}

impl GravityLensGlResources {
    /// 创建黑洞透镜 OpenGL 资源，适用于 glow backend paint callback。
    pub(crate) fn new(gl: Arc<glow::Context>) -> Result<Self, String> {
        let shader_version = egui_glow::ShaderVersion::get(&gl);
        let program = create_program(
            &gl,
            &gravity_vertex_shader(shader_version),
            &gravity_fragment_shader(shader_version),
        )?;
        let texture = unsafe {
            let texture = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
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
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
            texture
        };
        let (vertex_array, vertex_buffer) = create_quad(&gl)?;
        let (u_size, u_sampler, u_center, u_radius, u_time) = unsafe {
            (
                gl.get_uniform_location(program, "u_size"),
                gl.get_uniform_location(program, "u_sampler"),
                gl.get_uniform_location(program, "u_center"),
                gl.get_uniform_location(program, "u_radius"),
                gl.get_uniform_location(program, "u_time"),
            )
        };
        Ok(Self {
            gl,
            program,
            texture,
            vertex_array,
            vertex_buffer,
            u_size,
            u_sampler,
            u_center,
            u_radius,
            u_time,
            texture_width: 0,
            texture_height: 0,
        })
    }

    /// 拷贝当前 framebuffer 局部区域并用 shader 扭曲画回。
    pub(crate) fn paint(
        &mut self,
        gl: &glow::Context,
        info: egui::PaintCallbackInfo,
        lens: GravityLensPaint,
    ) {
        let viewport = info.viewport_in_pixels();
        if viewport.width_px == 0 || viewport.height_px == 0 {
            return;
        }
        let width = viewport.width_px as i32;
        let height = viewport.height_px as i32;
        self.ensure_texture_size(gl, width, height);

        unsafe {
            // 触发条件：后处理需要用当前已绘制 UI 作为输入。
            // 不能继续用旧截图 overlay：会出现圆形背景 patch。
            // 防止回归：原始线条和扭曲线条同时显示或出现灰色光圈。
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            gl.copy_tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                0,
                0,
                viewport.left_px,
                viewport.from_bottom_px,
                width,
                height,
            );
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.use_program(Some(self.program));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            gl.uniform_1_i32(self.u_sampler.as_ref(), 0);
            gl.uniform_2_f32(self.u_size.as_ref(), width as f32, height as f32);
            gl.uniform_2_f32(
                self.u_center.as_ref(),
                (lens.center.x - lens.rect.left()) * info.pixels_per_point,
                // 触发条件：copy_tex_sub_image_2d 读取的是 OpenGL 底部原点纹理。
                // 不能直接用 egui 的顶部原点 y：会把透镜中心垂直翻转。
                // 防止回归：黑洞中心偏离哈基米，背景看起来没有被影响。
                (lens.rect.bottom() - lens.center.y) * info.pixels_per_point,
            );
            gl.uniform_1_f32(self.u_radius.as_ref(), lens.radius * info.pixels_per_point);
            gl.uniform_1_f32(self.u_time.as_ref(), lens.time_seconds);
            gl.bind_vertex_array(Some(self.vertex_array));
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.bind_vertex_array(None);
            gl.use_program(None);
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }

    /// 保证局部 framebuffer 拷贝纹理尺寸足够当前透镜矩形。
    fn ensure_texture_size(&mut self, gl: &glow::Context, width: i32, height: i32) {
        if self.texture_width == width && self.texture_height == height {
            return;
        }
        self.texture_width = width;
        self.texture_height = height;
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                width,
                height,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }
}

impl Drop for GravityLensGlResources {
    /// 释放 OpenGL 资源，适用于 app 退出或资源被替换时。
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_buffer(self.vertex_buffer);
            self.gl.delete_vertex_array(self.vertex_array);
            self.gl.delete_texture(self.texture);
            self.gl.delete_program(self.program);
        }
    }
}

/// 可跨帧复用的黑洞透镜 OpenGL 状态。
#[derive(Default)]
pub(crate) struct GravityLensGlState {
    /// 实际 OpenGL 资源，首次 paint callback 时创建。
    resources: Mutex<Option<GravityLensGlResources>>,
}

impl GravityLensGlState {
    /// 创建空的黑洞透镜 OpenGL 状态。
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 执行一次黑洞透镜绘制，资源缺失时延迟初始化。
    pub(crate) fn paint(
        &self,
        painter: &egui_glow::Painter,
        info: egui::PaintCallbackInfo,
        lens: GravityLensPaint,
    ) {
        let mut resources = match self.resources.lock() {
            Ok(resources) => resources,
            Err(error) => {
                log::warn!("gravity lens shader state lock is poisoned: {error}");
                return;
            }
        };
        if resources.is_none() {
            match GravityLensGlResources::new(painter.gl().clone()) {
                Ok(created) => *resources = Some(created),
                Err(error) => {
                    log::warn!("failed to initialize gravity lens shader: {error}");
                    return;
                }
            }
        }
        if let Some(resources) = resources.as_mut() {
            resources.paint(painter.gl(), info, lens);
        }
    }
}

/// 构造可放入 egui painter 的 glow 回调。
pub(crate) fn gravity_lens_callback(
    state: Arc<GravityLensGlState>,
    lens: GravityLensPaint,
) -> egui::PaintCallback {
    egui::PaintCallback {
        rect: lens.rect,
        callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
            state.paint(painter, info, lens);
        })),
    }
}

/// 编译并链接 OpenGL shader program。
fn create_program(
    gl: &glow::Context,
    vertex: &str,
    fragment: &str,
) -> Result<glow::Program, String> {
    unsafe {
        let program = gl.create_program()?;
        let vertex_shader = compile_shader(gl, glow::VERTEX_SHADER, vertex)?;
        let fragment_shader = compile_shader(gl, glow::FRAGMENT_SHADER, fragment)?;
        gl.attach_shader(program, vertex_shader);
        gl.attach_shader(program, fragment_shader);
        // 触发条件：GLSL 120/100 ES 没有 layout(location)。
        // 不能依赖 driver 自动分配：VAO 固定把位置写到 0/1。
        // 防止回归：shader 成功链接但顶点属性读错导致后处理不可见。
        gl.bind_attrib_location(program, 0, "a_pos");
        gl.bind_attrib_location(program, 1, "a_uv");
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            let log = gl.get_program_info_log(program);
            gl.delete_shader(vertex_shader);
            gl.delete_shader(fragment_shader);
            gl.delete_program(program);
            return Err(log);
        }
        gl.detach_shader(program, vertex_shader);
        gl.detach_shader(program, fragment_shader);
        gl.delete_shader(vertex_shader);
        gl.delete_shader(fragment_shader);
        Ok(program)
    }
}

/// 编译一个 OpenGL shader。
fn compile_shader(gl: &glow::Context, kind: u32, source: &str) -> Result<glow::Shader, String> {
    unsafe {
        let shader = gl.create_shader(kind)?;
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            let log = gl.get_shader_info_log(shader);
            gl.delete_shader(shader);
            return Err(log);
        }
        Ok(shader)
    }
}

/// 创建覆盖 callback viewport 的四边形。
fn create_quad(gl: &glow::Context) -> Result<(glow::VertexArray, glow::Buffer), String> {
    let vertices: [f32; 16] = [
        -1.0, -1.0, 0.0, 0.0, 1.0, -1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0,
    ];
    let bytes = unsafe {
        std::slice::from_raw_parts(
            vertices.as_ptr().cast::<u8>(),
            vertices.len() * std::mem::size_of::<f32>(),
        )
    };
    unsafe {
        let vertex_array = gl.create_vertex_array()?;
        let vertex_buffer = gl.create_buffer()?;
        gl.bind_vertex_array(Some(vertex_array));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vertex_buffer));
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 16, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, 16, 8);
        gl.bind_buffer(glow::ARRAY_BUFFER, None);
        gl.bind_vertex_array(None);
        Ok((vertex_array, vertex_buffer))
    }
}

/// 生成匹配当前 glow context 的 vertex shader。
fn gravity_vertex_shader(shader_version: egui_glow::ShaderVersion) -> String {
    let interface = if shader_version.is_new_shader_interface() {
        r#"#define I in
#define O out
"#
    } else {
        r#"#define I attribute
#define O varying
"#
    };
    let precision = if shader_version.is_embedded() {
        "precision mediump float;\n"
    } else {
        ""
    };
    format!(
        "{}{}{}{}",
        shader_version.version_declaration(),
        precision,
        interface,
        GRAVITY_VERTEX_SHADER_BODY
    )
}

/// 生成匹配当前 glow context 的 fragment shader。
fn gravity_fragment_shader(shader_version: egui_glow::ShaderVersion) -> String {
    let interface = if shader_version.is_new_shader_interface() {
        r#"#define I in
out vec4 f_color;
#define OUT_COLOR f_color
#define TEXTURE texture
"#
    } else {
        r#"#define I varying
#define OUT_COLOR gl_FragColor
#define TEXTURE texture2D
"#
    };
    let precision = if shader_version.is_embedded() {
        "precision mediump float;\n"
    } else {
        ""
    };
    format!(
        "{}{}{}{}",
        shader_version.version_declaration(),
        precision,
        interface,
        GRAVITY_FRAGMENT_SHADER_BODY
    )
}

const GRAVITY_VERTEX_SHADER_BODY: &str = r#"
I vec2 a_pos;
I vec2 a_uv;
O vec2 v_uv;

void main() {
    v_uv = a_uv;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
"#;

const GRAVITY_FRAGMENT_SHADER_BODY: &str = r#"
uniform sampler2D u_sampler;
uniform vec2 u_size;
uniform vec2 u_center;
uniform float u_radius;
uniform float u_time;
I vec2 v_uv;

void main() {
    vec2 pos = v_uv * u_size;
    vec2 delta = pos - u_center;
    float distance = length(delta);
    if (distance >= u_radius || distance <= 0.5) {
        discard;
        return;
    }

    float radius_factor = 1.0 - distance / u_radius;
    float body_clear = min(u_radius * 0.28, 74.0);
    float body_fade = smoothstep(body_clear, body_clear + 44.0, distance);
    float edge_fade = 1.0 - smoothstep(u_radius * 0.72, u_radius, distance);
    float gravity = pow(radius_factor, 0.58) * edge_fade * body_fade;
    vec2 radial = delta / distance;
    vec2 tangent = vec2(-radial.y, radial.x);
    float pulse = sin(u_time * 1.4 + distance * 0.03) * 0.04;
    float pull = gravity * 104.0;
    float swirl = (gravity * 2.8 + pulse) * 34.0;
    vec2 sample_pos = pos + radial * pull + tangent * swirl;
    vec2 sample_uv = clamp(sample_pos / u_size, vec2(0.0), vec2(1.0));
    vec4 warped = TEXTURE(u_sampler, sample_uv);
    // 触发条件：后处理画在原 UI 上方，半透明混合会让原始线条残留。
    // 不能再采样 original 做颜色混合：会主动把未扭曲文字画回来。
    // 防止回归：透镜主体内背景没有被真正覆盖。
    float edge_alpha = 1.0 - smoothstep(u_radius * 0.68, u_radius, distance);
    float distortion_alpha = smoothstep(0.0, 0.08, gravity);
    float cover_alpha = edge_alpha * distortion_alpha;
    OUT_COLOR = vec4(warped.rgb * cover_alpha, cover_alpha);
}
"#;
