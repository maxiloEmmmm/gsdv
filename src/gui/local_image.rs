//! 本地图片解码、缓存与画廊状态。
//!
//! 文件读取和图片解码只在后台任务执行；GUI/TUI 绘制只读取已经解码的像素。

use eframe::egui;
use image::ImageReader;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 单张图片允许的最大文件大小。
pub(crate) const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
/// 单张图片允许的最大边长。
pub(crate) const MAX_IMAGE_DIMENSION: u32 = 4096;

/// 后台解码完成、可供两种前端消费的图片。
#[derive(Debug, Clone)]
pub(crate) struct DecodedLocalImage {
    /// 规范化后的绝对文件路径。
    pub path: PathBuf,
    /// 像素宽度。
    pub width: usize,
    /// 像素高度。
    pub height: usize,
    /// RGBA8 像素，GUI 上传纹理、TUI 生成半块字符时共用。
    pub rgba: Arc<Vec<u8>>,
}

/// GUI Markdown block 引用的轻量纹理信息。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LocalImageRef {
    /// egui 纹理 id。
    pub texture_id: egui::TextureId,
    /// 原图宽度。
    pub width: usize,
    /// 原图高度。
    pub height: usize,
}

/// 已加载图片及其 GUI 纹理生命周期 owner。
pub(crate) struct ReadyLocalImage {
    /// 两种前端共用的解码像素。
    pub decoded: DecodedLocalImage,
    /// GUI 使用的纹理句柄。
    pub texture: egui::TextureHandle,
}

/// 单个路径的加载状态。
pub(crate) enum LocalImageEntry {
    /// 后台任务正在读取或解码。
    Loading,
    /// 图片已经可绘制。
    Ready(ReadyLocalImage),
    /// 最近一次加载失败的原因。
    Failed(String),
}

/// 进程级本地图片缓存。
#[derive(Default)]
pub(crate) struct LocalImageStore {
    /// 绝对路径到加载状态。
    entries: BTreeMap<PathBuf, LocalImageEntry>,
    /// 加载完成后需要加入画廊的路径。
    gallery_waiters: BTreeSet<PathBuf>,
}

impl LocalImageStore {
    /// 判断路径是否已经进入缓存生命周期。
    ///
    /// 适用场景：render 只为首次出现的 Markdown 图片投递 AppEvent。示例：Loading -> true。
    pub(crate) fn contains(&self, path: &Path) -> bool {
        self.entries.contains_key(path)
    }

    /// 标记路径正在加载，返回是否需要真正派发后台任务。
    ///
    /// 适用场景：多个 Markdown 或 hook 同时引用同图。示例：第二次请求 -> false。
    pub(crate) fn begin_request(&mut self, path: &Path, gallery: bool) -> bool {
        if gallery {
            self.gallery_waiters.insert(path.to_path_buf());
        }
        match self.entries.get(path) {
            Some(LocalImageEntry::Loading | LocalImageEntry::Ready(_)) => false,
            Some(LocalImageEntry::Failed(_)) | None => {
                self.entries
                    .insert(path.to_path_buf(), LocalImageEntry::Loading);
                true
            }
        }
    }

    /// 合并后台结果并在成功时创建 GUI 纹理。
    ///
    /// 适用场景：`AppEvent::LocalImageLoaded`。示例：RGBA -> Ready。
    pub(crate) fn finish_request(
        &mut self,
        ctx: &egui::Context,
        path: PathBuf,
        result: Result<DecodedLocalImage, String>,
    ) -> bool {
        let gallery = self.gallery_waiters.remove(&path);
        let entry = match result {
            Ok(decoded) => {
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [decoded.width, decoded.height],
                    decoded.rgba.as_slice(),
                );
                let texture = ctx.load_texture(
                    format!("local-image:{}", path.display()),
                    color,
                    egui::TextureOptions::LINEAR,
                );
                LocalImageEntry::Ready(ReadyLocalImage { decoded, texture })
            }
            Err(error) => LocalImageEntry::Failed(error),
        };
        self.entries.insert(path, entry);
        gallery
    }

    /// 返回 GUI Markdown 使用的纹理快照。
    ///
    /// 适用场景：加载完成后装饰 Markdown block。示例：Ready -> Some(ref)。
    pub(crate) fn image_ref(&self, path: &Path) -> Option<LocalImageRef> {
        let LocalImageEntry::Ready(ready) = self.entries.get(path)? else {
            return None;
        };
        Some(LocalImageRef {
            texture_id: ready.texture.id(),
            width: ready.decoded.width,
            height: ready.decoded.height,
        })
    }

    /// 返回 TUI 使用的已解码像素。
    ///
    /// 适用场景：ANSI 半块字符绘制。示例：Ready -> RGBA。
    pub(crate) fn ready(&self, path: &Path) -> Option<&ReadyLocalImage> {
        match self.entries.get(path)? {
            LocalImageEntry::Ready(ready) => Some(ready),
            LocalImageEntry::Loading | LocalImageEntry::Failed(_) => None,
        }
    }

    /// 返回图片加载错误。
    ///
    /// 适用场景：预览显示明确错误。示例：超限 -> 错误字符串。
    pub(crate) fn error(&self, path: &Path) -> Option<&str> {
        match self.entries.get(path)? {
            LocalImageEntry::Failed(error) => Some(error),
            LocalImageEntry::Loading | LocalImageEntry::Ready(_) => None,
        }
    }
}

/// 顶层图片画廊的纯内存状态。
#[derive(Default)]
pub(crate) struct ImageGalleryState {
    /// 本次打开期间收到的图片路径。
    pub paths: Vec<PathBuf>,
    /// 当前显示索引。
    pub current: usize,
    /// 是否显示顶层画廊。
    pub open: bool,
    /// 当前图片未切换时到达的新图数量。
    pub unseen: usize,
}

impl ImageGalleryState {
    /// 追加图片并打开画廊，不抢走用户正在查看的当前项。
    ///
    /// 适用场景：连续 `image.show`。示例：已看第 1 张 + 新图 -> 保持第 1 张。
    pub(crate) fn push(&mut self, path: PathBuf) {
        if self.paths.contains(&path) {
            self.open = true;
            return;
        }
        if self.open && !self.paths.is_empty() {
            self.unseen = self.unseen.saturating_add(1);
        }
        self.paths.push(path);
        self.open = true;
        self.current = self.current.min(self.paths.len().saturating_sub(1));
    }

    /// 关闭并清空本次画廊队列。
    ///
    /// 适用场景：用户点击 Close。示例：3 张 -> 空队列。
    pub(crate) fn close(&mut self) {
        *self = Self::default();
    }

    /// 切换到相邻图片并清除新图提示。
    ///
    /// 适用场景：左右按钮。示例：最后一张 Next -> 第一张。
    pub(crate) fn move_by(&mut self, delta: isize) {
        let len = self.paths.len();
        if len == 0 {
            return;
        }
        self.current = (self.current as isize + delta).rem_euclid(len as isize) as usize;
        self.unseen = 0;
    }
}

/// 将 Markdown 图片地址解析为本机绝对路径。
///
/// 适用场景：绝对路径或相对当前 Markdown 文件。示例：`docs/a.md + x.png -> docs/x.png`。
pub(crate) fn resolve_markdown_image(markdown_file: &Path, source: &str) -> Option<PathBuf> {
    if source.starts_with("http://")
        || source.starts_with("https://")
        || source.starts_with("data:")
    {
        return None;
    }
    let source = source.strip_prefix("file://").unwrap_or(source);
    let path = PathBuf::from(source);
    let absolute = if path.is_absolute() {
        path
    } else {
        markdown_file.parent()?.join(path)
    };
    Some(absolute.canonicalize().unwrap_or(absolute))
}

/// 在后台线程读取并解码一张本地图片。
///
/// 适用场景：AppEvent 派发后的 blocking task。示例：PNG -> RGBA8。
pub(crate) fn decode(path: PathBuf) -> Result<DecodedLocalImage, String> {
    let metadata = std::fs::metadata(&path)
        .map_err(|error| format!("Failed to inspect {}: {error}", path.display()))?;
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err(format!("Image exceeds 20 MiB: {}", path.display()));
    }
    let file =
        File::open(&path).map_err(|error| format!("Failed to open {}: {error}", path.display()))?;
    let reader = ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .map_err(|error| format!("Failed to detect {}: {error}", path.display()))?;
    let image = reader
        .decode()
        .map_err(|error| format!("Failed to decode {}: {error}", path.display()))?;
    if image.width() > MAX_IMAGE_DIMENSION || image.height() > MAX_IMAGE_DIMENSION {
        return Err(format!("Image exceeds 4096x4096: {}", path.display()));
    }
    let rgba = image.into_rgba8();
    Ok(DecodedLocalImage {
        path,
        width: rgba.width() as usize,
        height: rgba.height() as usize,
        rgba: Arc::new(rgba.into_raw()),
    })
}
