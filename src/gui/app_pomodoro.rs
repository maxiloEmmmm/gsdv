//! 番茄钟浮层和休息动画。
//!
//! 这里承接番茄钟 overlay 绘制和会话内动画状态更新；计时状态机
//! 的事件推进仍由 AppEvent drain 触发。

use super::*;

impl GsdvGuiApp {
    /// 绘制工作末段的哈基米预警动画。
    pub(super) fn pomodoro_work_peek_overlay(&mut self, ctx: &egui::Context) {
        if !self.runtime_settings.pomodoro_enabled || self.pomodoro.phase != PomodoroPhase::Working
        {
            return;
        }
        let Some(texture) = self.hajimi_texture(ctx) else {
            return;
        };
        let progress = pomodoro_work_progress(&self.runtime_settings, &self.pomodoro);
        let warning_progress = pomodoro_warning_progress(&self.runtime_settings);
        if progress < warning_progress {
            return;
        }
        let reveal = ((progress - warning_progress) / (1.0 - warning_progress)).clamp(0.0, 1.0);
        let screen = ctx.screen_rect();
        let now = Instant::now();
        let elapsed = now.duration_since(self.pomodoro.phase_started_at);
        let pos = self.pomodoro.cat_pos;
        self.animate_pomodoro_cat(screen, now);

        egui::Area::new("pomodoro-work-peek-overlay".into())
            .order(egui::Order::Tooltip)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(POMODORO_CAT_SIZE, Sense::hover());
                draw_hajimi_work_warning_cat(ui, rect, &texture, reveal, elapsed);
            });
        self.request_app_repaint();
    }

    /// 绘制休息相关阶段的哈基米。
    pub(super) fn pomodoro_cat_overlay(&mut self, ctx: &egui::Context) {
        if !self.runtime_settings.pomodoro_enabled || self.pomodoro.phase == PomodoroPhase::Working
        {
            return;
        }
        let now = Instant::now();
        let screen = ctx.screen_rect();
        let phase = self.pomodoro.phase;
        let phase_elapsed = if phase == PomodoroPhase::WaitingForRestQuiet {
            now.duration_since(self.pomodoro.rest_quiet_animation_started_at)
        } else {
            now.duration_since(self.pomodoro.phase_started_at)
        };
        let has_gravity_lens = matches!(
            phase,
            PomodoroPhase::WaitingForRestQuiet | PomodoroPhase::Resting
        );

        let Some(texture) = self.hajimi_texture(ctx) else {
            return;
        };
        let area_pos = self.pomodoro.cat_pos;

        egui::Area::new("pomodoro-foreground-overlay".into())
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(0.0, 0.0))
            .show(ctx, |ui| {
                if has_gravity_lens {
                    self.paint_pomodoro_gravity_lens(ui, screen, now);
                }
                let cat_rect = Rect::from_min_size(area_pos, POMODORO_CAT_SIZE);
                draw_hajimi_cat(ui, cat_rect, &texture, phase, phase_elapsed);
            });

        if matches!(
            phase,
            PomodoroPhase::WaitingForRestQuiet | PomodoroPhase::Resting
        ) {
            self.animate_pomodoro_cat(screen, now);
        } else {
            self.pomodoro.last_animation_at = now;
        }

        self.update_pomodoro_meows(now);
        if self.pomodoro.phase == PomodoroPhase::Resting {
            self.pomodoro_meow_overlay(ctx, now);
        }
    }

    /// 在哈基米前景层内插入当前 framebuffer 黑洞后处理。
    pub(super) fn paint_pomodoro_gravity_lens(
        &self,
        ui: &mut egui::Ui,
        screen: Rect,
        now: Instant,
    ) {
        let center = self.pomodoro.cat_pos + Vec2::new(126.0, 116.0);
        let rect = Rect::from_center_size(center, Vec2::splat(POMODORO_GRAVITY_LENS_RADIUS * 2.0))
            .intersect(screen);
        if rect.is_negative() || rect.width() < 4.0 || rect.height() < 4.0 {
            return;
        }

        let painter = ui.painter();
        let elapsed = now.duration_since(self.pomodoro.phase_started_at);
        painter.add(egui::Shape::Callback(
            crate::gui::glow_gravity_lens::gravity_lens_callback(
                self.pomodoro_gravity_lens_gl.clone(),
                crate::gui::glow_gravity_lens::GravityLensPaint {
                    rect,
                    center,
                    radius: POMODORO_GRAVITY_LENS_RADIUS,
                    time_seconds: elapsed.as_secs_f32(),
                },
            ),
        ));
    }

    /// 为番茄钟浮层加载一次哈基米贴图。
    pub(super) fn hajimi_texture(&mut self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        if self.hajimi_texture.is_none() {
            let image = hajimi_color_image()?;
            self.hajimi_texture =
                Some(ctx.load_texture("pomodoro-hajimi", image, egui::TextureOptions::NEAREST));
        }
        self.hajimi_texture.clone()
    }

    /// 按休息模式的弹跳路径移动哈基米。
    pub(super) fn animate_pomodoro_cat(&mut self, screen: Rect, now: Instant) {
        let dt = now
            .saturating_duration_since(self.pomodoro.last_animation_at)
            .as_secs_f32()
            .min(0.08);
        self.pomodoro.last_animation_at = now;
        self.pomodoro.cat_pos += self.pomodoro.cat_velocity * dt;
        let min_x = screen.left() + 12.0;
        let min_y = screen.top() + 44.0;
        let max_x = (screen.right() - POMODORO_CAT_SIZE.x - 12.0).max(min_x);
        let max_y = (screen.bottom() - POMODORO_CAT_SIZE.y - BOTTOM_BAR_HEIGHT - 12.0).max(min_y);

        if self.pomodoro.cat_pos.x <= min_x || self.pomodoro.cat_pos.x >= max_x {
            self.pomodoro.cat_velocity.x = -self.pomodoro.cat_velocity.x;
        }
        if self.pomodoro.cat_pos.y <= min_y || self.pomodoro.cat_pos.y >= max_y {
            self.pomodoro.cat_velocity.y = -self.pomodoro.cat_velocity.y;
        }
        self.pomodoro.cat_pos.x = self.pomodoro.cat_pos.x.clamp(min_x, max_x);
        self.pomodoro.cat_pos.y = self.pomodoro.cat_pos.y.clamp(min_y, max_y);
    }

    /// Adds and expires floating meow text for the resting cat.
    pub(super) fn update_pomodoro_meows(&mut self, now: Instant) {
        self.pomodoro
            .meows
            .retain(|meow| now.duration_since(meow.created_at) < Duration::from_millis(1400));
        if self.pomodoro.phase != PomodoroPhase::Resting || now < self.pomodoro.next_meow_at {
            return;
        }
        self.pomodoro.meows.push(PomodoroMeow {
            origin: self.pomodoro.cat_pos + Vec2::new(78.0, 24.0),
            created_at: now,
        });
        self.pomodoro.next_meow_at = now + POMODORO_MEOW_INTERVAL;
    }

    /// Paints floating meow texts above every panel.
    pub(super) fn pomodoro_meow_overlay(&self, ctx: &egui::Context, now: Instant) {
        if self.pomodoro.meows.is_empty() {
            return;
        }
        egui::Area::new("pomodoro-meow-overlay".into())
            .order(egui::Order::Tooltip)
            .fixed_pos(egui::pos2(0.0, 0.0))
            .show(ctx, |ui| {
                let painter = ui.painter();
                let rest_remaining =
                    pomodoro_rest_remaining(&self.runtime_settings, &self.pomodoro, now);
                let label = i18n::text_with_arg(
                    self.app_language,
                    "Resting {time}",
                    "{time}",
                    format_minutes_seconds(rest_remaining),
                );
                for meow in &self.pomodoro.meows {
                    let age = now.duration_since(meow.created_at).as_secs_f32();
                    let pos = meow.origin + Vec2::new(age * 12.0, -age * 32.0);
                    let alpha = ((1.0 - age / 1.4).clamp(0.0, 1.0) * 255.0) as u8;
                    painter.text(
                        pos,
                        Align2::CENTER_CENTER,
                        &label,
                        egui::FontId::new(18.0, theme::editor_system_font_family()),
                        Color32::from_rgba_unmultiplied(0xD9, 0x77, 0x06, alpha),
                    );
                }
            });
    }
}
