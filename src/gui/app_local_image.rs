//! 本地图片 AppEvent 派发、Markdown 装饰与 GUI 画廊。

use super::*;

impl GsdvGuiApp {
    /// 为 Workflow 片段准备带本地图片引用的 Markdown blocks。
    ///
    /// 适用场景：GUI task/step Preview。示例：`task.md + ./a.png` -> 共享纹理引用。
    pub(super) fn workflow_preview_blocks(
        &self,
        markdown: &str,
        task_path: &Path,
    ) -> Vec<markdown_preview::MarkdownBlock> {
        let mut blocks = markdown_preview::parse(markdown);
        let Some(workspace) = self.current_workspace() else {
            return blocks;
        };
        let markdown_file = workspace.path.join(task_path);
        let paths = markdown_preview::image_sources(&blocks)
            .into_iter()
            .filter_map(|source| local_image::resolve_markdown_image(&markdown_file, source))
            .collect::<BTreeSet<_>>();
        for path in paths {
            if !self.local_images.contains(&path) {
                self.queue_app_event(AppEvent::RequestLocalImage {
                    path,
                    gallery: false,
                });
            }
        }
        markdown_preview::decorate_images(&mut blocks, &markdown_file, &self.local_images);
        blocks
    }

    /// 请求本地图片；命中缓存时只处理画廊，不重复读取文件。
    ///
    /// 适用场景：Markdown 或 `image.show`。示例：同一路径第二次请求 -> 复用缓存。
    pub(super) fn request_local_image(&mut self, path: PathBuf, gallery: bool) {
        if self.local_images.ready(&path).is_some() {
            if gallery {
                self.image_gallery.push(path);
                self.request_app_repaint();
            }
            return;
        }
        if !self.local_images.begin_request(&path, gallery) {
            return;
        }
        let tx = self.app_event_tx.clone();
        let repaint_ctx = self.app_repaint_ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        let event_path = path.clone();
        self.background_runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || local_image::decode(path))
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result);
            let _ = tx.send(AppEvent::LocalImageLoaded {
                path: event_path,
                result,
            });
            if let Some(ctx) = repaint_ctx {
                repaint_controller.request_repaint(&ctx);
            }
        });
    }

    /// 扫描指定文档的图片 block，并派发尚未缓存的图片。
    ///
    /// 适用场景：文档加载或 Markdown 重解析完成。示例：两张相对图片 -> 两个去重请求。
    pub(super) fn request_document_local_images(&mut self, index: usize) {
        let Some(document) = self.documents.get(index) else {
            return;
        };
        let Some(path) = document.absolute_path.as_ref() else {
            return;
        };
        let sources = markdown_preview::image_sources(&document.markdown_preview_blocks);
        let paths = sources
            .into_iter()
            .filter_map(|source| local_image::resolve_markdown_image(path, source))
            .collect::<BTreeSet<_>>();
        for path in paths {
            self.request_local_image(path, false);
        }
        self.decorate_document_images(index);
    }

    /// 把缓存纹理引用合并进文档 Markdown blocks。
    ///
    /// 适用场景：图片加载完成或文档重解析。示例：source -> TextureId。
    pub(super) fn decorate_document_images(&mut self, index: usize) {
        let Some(document) = self.documents.get_mut(index) else {
            return;
        };
        let Some(markdown_path) = document.absolute_path.as_ref() else {
            return;
        };
        markdown_preview::decorate_images(
            &mut document.markdown_preview_blocks,
            markdown_path,
            &self.local_images,
        );
        document.markdown_preview_metrics = None;
        document.markdown_preview_metrics_width = 0.0;
    }

    /// 合并图片后台结果，并同步 Markdown 与画廊可见状态。
    ///
    /// 适用场景：`AppEvent::LocalImageLoaded`。示例：画廊 waiter -> 自动打开。
    pub(super) fn apply_local_image_loaded(
        &mut self,
        ctx: &egui::Context,
        path: PathBuf,
        result: Result<local_image::DecodedLocalImage, String>,
    ) {
        let gallery = self.local_images.finish_request(ctx, path.clone(), result);
        for index in 0..self.documents.len() {
            self.decorate_document_images(index);
        }
        if gallery && self.local_images.ready(&path).is_some() {
            self.image_gallery.push(path);
        }
        self.request_app_repaint();
    }

    /// 绘制 GUI 顶层图片画廊。
    ///
    /// 适用场景：socket 连续展示图片。示例：3 张 -> 左右翻页并提示新图。
    pub(super) fn image_gallery_overlay(&mut self, ctx: &egui::Context) {
        if !self.image_gallery.open {
            return;
        }
        let mut open = true;
        let mut move_by = 0_isize;
        let mut close = false;
        let current = self
            .image_gallery
            .paths
            .get(self.image_gallery.current)
            .cloned();
        egui::Window::new("Image Gallery")
            .id(egui::Id::new("gsdv-image-gallery"))
            .collapsible(false)
            .resizable(true)
            .default_size(Vec2::new(920.0, 680.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("<").clicked() {
                        move_by = -1;
                    }
                    ui.label(format!(
                        "{}/{}",
                        self.image_gallery.current.saturating_add(1),
                        self.image_gallery.paths.len()
                    ));
                    if ui.button(">").clicked() {
                        move_by = 1;
                    }
                    if self.image_gallery.unseen > 0 {
                        ui.colored_label(
                            theme::warning(),
                            format!("{} new", self.image_gallery.unseen),
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                    });
                });
                ui.separator();
                if let Some(path) = current.as_ref() {
                    ui.label(path.display().to_string());
                    if let Some(image) = self.local_images.ready(path) {
                        let available = ui.available_size();
                        let original =
                            Vec2::new(image.decoded.width as f32, image.decoded.height as f32);
                        let scale = (available.x / original.x)
                            .min(available.y / original.y)
                            .min(1.0)
                            .max(0.01);
                        ui.centered_and_justified(|ui| {
                            ui.add(egui::Image::new((image.texture.id(), original * scale)));
                        });
                    } else if let Some(error) = self.local_images.error(path) {
                        ui.colored_label(theme::danger(), error);
                    } else {
                        ui.spinner();
                    }
                }
            });
        if move_by != 0 {
            self.image_gallery.move_by(move_by);
        }
        if close || !open {
            self.image_gallery.close();
        }
    }
}
