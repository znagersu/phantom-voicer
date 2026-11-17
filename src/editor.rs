use crate::harmony::SharedUiState;
use crate::params::{KeyRoot, PhantomParams, ScaleMode, SoundPatch, StrumDir, VoicingStyle};
use crate::updater::{UpdateStatus, UpdaterHandle, CURRENT_VERSION};
use nih_plug::prelude::*;
use nih_plug_egui::{
    create_egui_editor,
    egui::{
        self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Pos2, Rect,
        RichText, Sense, Shape, Stroke, StrokeKind, Vec2,
    },
};
use std::f32::consts::{PI, TAU};
use std::sync::{atomic::Ordering, Arc};

const P5_BG: Color32 = Color32::from_rgb(11, 11, 14);
const P5_PANEL: Color32 = Color32::from_rgb(19, 19, 24);
const P5_RED: Color32 = Color32::from_rgb(230, 0, 26);
const P5_RED_DEEP: Color32 = Color32::from_rgb(140, 4, 18);
const P5_WHITE: Color32 = Color32::from_rgb(248, 248, 250);
const P5_MUTED: Color32 = Color32::from_rgb(184, 184, 200);
const P5_GOLD: Color32 = Color32::from_rgb(247, 184, 1);
const P5_CYAN: Color32 = Color32::from_rgb(28, 232, 255);
const P5_MAGENTA: Color32 = Color32::from_rgb(255, 62, 165);
const P5_BLACK: Color32 = Color32::from_rgb(0, 0, 0);

const FONT_IMPACT: &str = "p5_impact";
const FONT_HEAVY: &str = "p5_heavy";
const FONT_SERIF: &str = "p5_serif";
const FONT_COMIC: &str = "p5_comic";

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum UiTab {
    #[default]
    MainRack,
    Mark1Kore,
}

#[derive(Default)]
struct EditorUiState {
    auditioning_note: Option<u8>,
    active_tab: UiTab,
}

#[derive(Clone, Copy)]
enum P5Icon {
    Square,
    Triangle,
    Circle,
    Cross,
}

fn setup_p5_fonts(egui_ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    let load_win_font = |fonts: &mut FontDefinitions, key: &str, filename: &str| -> bool {
        let path = format!("C:\\Windows\\Fonts\\{filename}");
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert(key.to_owned(), Arc::new(FontData::from_owned(bytes)));
            let mut fam = vec![key.to_owned()];
            if let Some(default_prop) = fonts.families.get(&FontFamily::Proportional) {
                fam.extend(default_prop.iter().cloned());
            }
            fonts
                .families
                .insert(FontFamily::Name(key.into()), fam);
            true
        } else {
            false
        }
    };

    let has_impact = load_win_font(&mut fonts, FONT_IMPACT, "impact.ttf");
    let _ = load_win_font(&mut fonts, FONT_HEAVY, "ariblk.ttf");
    let _ = load_win_font(&mut fonts, FONT_SERIF, "georgiab.ttf");
    let _ = load_win_font(&mut fonts, FONT_COMIC, "comicbd.ttf");

    // Make Impact / Arial Black fallback cleanly if any family wasn't loaded
    if !has_impact {
        if let Some(prop) = fonts.families.get(&FontFamily::Proportional).cloned() {
            fonts
                .families
                .insert(FontFamily::Name(FONT_IMPACT.into()), prop);
        }
    }

    egui_ctx.set_fonts(fonts);
}

#[inline]
fn char_hash(idx: usize, ch: char) -> u32 {
    let mut x = (idx as u32).wrapping_mul(0x9E37_79B9) ^ (ch as u32).wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^ (x >> 16)
}

/// Draws true Persona 5 contour-hugging 3D sticker typography:
/// 1) Fused outer rim polygons per-character (creates spiky comic cutout contour around ascenders/descenders)
/// 2) Fused inner black cutout polygons + 3D depth shadow
/// 3) Per-character vertical bounce, slight scale variation, tight kerning & 3D black glyph extrusion
/// 4) Crisp bright foreground glyph on top
#[allow(clippy::too_many_arguments)]
fn draw_p5_3d_text(
    painter: &egui::Painter,
    pos: Pos2,
    align: egui::Align2,
    text: &str,
    base_size: f32,
    fg_color: Color32,
    rim_color: Color32,
    family_name: &str,
) -> Rect {
    let font_fam = FontFamily::Name(family_name.into());

    struct CharGlyph {
        ch: char,
        rect: Rect,
        dy: f32,
        tilt_top: f32,
        tilt_bot: f32,
        font: FontId,
    }

    let mut items: Vec<CharGlyph> = Vec::with_capacity(text.len());
    let mut cursor_x = 0.0_f32;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;

    for (idx, ch) in text.chars().enumerate() {
        if ch == ' ' {
            cursor_x += base_size * 0.28;
            continue;
        }

        let h = char_hash(idx, ch);
        let is_lead = idx == 0 || text.chars().nth(idx.saturating_sub(1)) == Some(' ');
        let scale = if is_lead {
            1.12
        } else {
            0.94 + ((h & 0xFF) as f32 / 255.0) * 0.12
        };
        let dy = (((h >> 8) & 0xFF) as f32 / 255.0 - 0.5) * (base_size * 0.14);
        let tilt_top = (((h >> 16) & 0xFF) as f32 / 255.0 - 0.5) * 3.5;
        let tilt_bot = (((h >> 24) & 0xFF) as f32 / 255.0 - 0.5) * 3.5;

        let font = FontId::new(base_size * scale, font_fam.clone());
        let g = painter.layout_no_wrap(ch.to_string(), font.clone(), fg_color);
        let gsize = g.size();

        let r = Rect::from_min_size(Pos2::new(cursor_x, dy - gsize.y * 0.5), gsize);
        min_y = min_y.min(r.top());
        max_y = max_y.max(r.bottom());

        items.push(CharGlyph {
            ch,
            rect: r,
            dy,
            tilt_top,
            tilt_bot,
            font,
        });

        // Tight comic overlap so letters nest together like Persona 5 UI words
        cursor_x += (gsize.x * 0.90).max(base_size * 0.24);
    }

    if items.is_empty() {
        return Rect::from_center_size(pos, Vec2::ZERO);
    }

    let total_w = items.last().map(|i| i.rect.right()).unwrap_or(0.0);
    let total_h = (max_y - min_y).max(base_size);
    let anchor = align.anchor_size(pos, Vec2::new(total_w, total_h));
    let offset = Vec2::new(anchor.left(), anchor.top() - min_y);

    // Pass 1: 3D depth drop shadow + outer sticker rim (white/red) fused across all chars
    let rim_pad = Vec2::new((base_size * 0.22).max(3.5), (base_size * 0.16).max(2.6));
    for item in &items {
        let r = item.rect.translate(offset).expand2(rim_pad);
        let poly = vec![
            Pos2::new(r.left() + item.tilt_top, r.top() - 1.0),
            Pos2::new(r.right() + item.tilt_top, r.top() + 0.5),
            Pos2::new(r.right() - item.tilt_bot, r.bottom() + 1.0),
            Pos2::new(r.left() - item.tilt_bot, r.bottom() - 0.5),
        ];
        // Hard 3D shadow drop behind sticker
        let shadow: Vec<Pos2> = poly.iter().map(|p| *p + Vec2::new(2.5, 2.8)).collect();
        painter.add(Shape::convex_polygon(shadow, P5_BLACK, Stroke::NONE));
        painter.add(Shape::convex_polygon(poly, rim_color, Stroke::NONE));
    }

    // Pass 2: Fused heavy BLACK inner sticker silhouette (leaves a jagged 2px rim around the contour)
    let inner_pad = Vec2::new((base_size * 0.13).max(2.0), (base_size * 0.08).max(1.4));
    for item in &items {
        let r = item.rect.translate(offset).expand2(inner_pad);
        let poly = vec![
            Pos2::new(r.left() + item.tilt_top, r.top() - 0.5),
            Pos2::new(r.right() + item.tilt_top, r.top() + 0.5),
            Pos2::new(r.right() - item.tilt_bot, r.bottom() + 0.5),
            Pos2::new(r.left() - item.tilt_bot, r.bottom() - 0.5),
        ];
        painter.add(Shape::convex_polygon(poly, P5_BLACK, Stroke::NONE));
    }

    // Pass 3: 3D black glyph extrusion + crisp thick outline
    let extrude_offsets = [
        Vec2::new(2.6, 2.8),
        Vec2::new(1.8, 2.0),
        Vec2::new(1.0, 1.2),
        Vec2::new(-1.2, 0.0),
        Vec2::new(1.2, 0.0),
        Vec2::new(0.0, -1.2),
        Vec2::new(0.0, 1.2),
    ];
    for item in &items {
        let ch_pos = item.rect.min + offset;
        let _ = item.dy;
        for off in extrude_offsets {
            let g_blk = painter.layout_no_wrap(item.ch.to_string(), item.font.clone(), P5_BLACK);
            painter.galley(ch_pos + off, g_blk, P5_BLACK);
        }
    }

    // Pass 4: Crisp foreground bright glyphs
    for item in &items {
        let ch_pos = item.rect.min + offset;
        let g_fg = painter.layout_no_wrap(item.ch.to_string(), item.font.clone(), fg_color);
        painter.galley(ch_pos, g_fg, fg_color);
    }

    anchor.expand2(rim_pad)
}

/// Draws mixed-font, tilted alternating cutout-tile + bubble-outline typography
/// directly inspired by the Persona 5 Menu Font sheet.
fn draw_p5_ransom_word(
    painter: &egui::Painter,
    pos: Pos2,
    align: egui::Align2,
    text: &str,
    base_size: f32,
    accent: Color32,
) -> Rect {
    let families = [FONT_HEAVY, FONT_SERIF, FONT_IMPACT, FONT_COMIC];

    struct RansomGlyph {
        ch: char,
        rect: Rect,
        style_kind: u32,
        tilt_top: f32,
        tilt_bot: f32,
        font: FontId,
    }

    let mut items: Vec<RansomGlyph> = Vec::with_capacity(text.len());
    let mut cx = 0.0_f32;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;

    for (idx, ch) in text.chars().enumerate() {
        if ch == ' ' {
            cx += base_size * 0.34;
            continue;
        }
        let h = char_hash(idx + 17, ch);
        let fam = families[(h as usize) % families.len()];
        let scale = 0.90 + (((h >> 4) & 0xFF) as f32 / 255.0) * 0.22;
        let dy = (((h >> 12) & 0xFF) as f32 / 255.0 - 0.5) * (base_size * 0.16);
        let tilt_top = (((h >> 20) & 0xFF) as f32 / 255.0 - 0.5) * 4.5;
        let tilt_bot = (((h >> 24) & 0xFF) as f32 / 255.0 - 0.5) * 4.5;
        let style_kind = (h >> 2) % 4; // 0: black tile, 1: white tile, 2: accent tile, 3: bubble outline

        let font = FontId::new(base_size * scale, FontFamily::Name(fam.into()));
        let g = painter.layout_no_wrap(ch.to_string(), font.clone(), P5_WHITE);
        let gsize = g.size();
        let r = Rect::from_min_size(Pos2::new(cx, dy - gsize.y * 0.5), gsize);
        min_y = min_y.min(r.top());
        max_y = max_y.max(r.bottom());

        items.push(RansomGlyph {
            ch,
            rect: r,
            style_kind,
            tilt_top,
            tilt_bot,
            font,
        });
        cx += (gsize.x + 4.0).max(base_size * 0.52);
    }

    if items.is_empty() {
        return Rect::from_center_size(pos, Vec2::ZERO);
    }

    let total_w = items.last().map(|i| i.rect.right()).unwrap_or(0.0);
    let total_h = (max_y - min_y).max(base_size);
    let anchor = align.anchor_size(pos, Vec2::new(total_w, total_h));
    let offset = Vec2::new(anchor.left(), anchor.top() - min_y);

    for item in &items {
        let r = item.rect.translate(offset);
        let box_r = r.expand2(Vec2::new(3.0, 2.0));
        let poly = vec![
            Pos2::new(box_r.left() + item.tilt_top, box_r.top() - 1.0),
            Pos2::new(box_r.right() + item.tilt_top, box_r.top() + 1.0),
            Pos2::new(box_r.right() - item.tilt_bot, box_r.bottom() + 1.0),
            Pos2::new(box_r.left() - item.tilt_bot, box_r.bottom() - 1.0),
        ];
        let shadow: Vec<Pos2> = poly.iter().map(|p| *p + Vec2::new(2.0, 2.2)).collect();

        match item.style_kind {
            0 => {
                // Solid black cutout tile with white border & crisp white glyph
                painter.add(Shape::convex_polygon(shadow, P5_BLACK, Stroke::NONE));
                painter.add(Shape::convex_polygon(
                    poly,
                    P5_BLACK,
                    Stroke::new(1.5_f32, P5_WHITE),
                ));
                let g = painter.layout_no_wrap(item.ch.to_string(), item.font.clone(), P5_WHITE);
                painter.galley(r.min, g, P5_WHITE);
            }
            1 => {
                // Crisp white cutout tile with thick black border & black/accent glyph
                painter.add(Shape::convex_polygon(shadow, P5_BLACK, Stroke::NONE));
                painter.add(Shape::convex_polygon(
                    poly,
                    P5_WHITE,
                    Stroke::new(2.0_f32, P5_BLACK),
                ));
                let fg = P5_BLACK;
                let g = painter.layout_no_wrap(item.ch.to_string(), item.font.clone(), fg);
                painter.galley(r.min, g, fg);
            }
            2 => {
                // Accent (crimson/gold) cutout tile with black border & white glyph
                painter.add(Shape::convex_polygon(shadow, P5_BLACK, Stroke::NONE));
                painter.add(Shape::convex_polygon(
                    poly,
                    accent,
                    Stroke::new(1.8_f32, P5_BLACK),
                ));
                let fg = if accent == P5_GOLD { P5_BLACK } else { P5_WHITE };
                let g = painter.layout_no_wrap(item.ch.to_string(), item.font.clone(), fg);
                painter.galley(r.min, g, fg);
            }
            _ => {
                // Bubble-outlined standalone letter (white inside thick black 3D outline, no tile)
                for off in [
                    Vec2::new(2.4, 2.4),
                    Vec2::new(-1.8, 0.0),
                    Vec2::new(1.8, 0.0),
                    Vec2::new(0.0, -1.8),
                    Vec2::new(0.0, 1.8),
                    Vec2::new(-1.3, -1.3),
                    Vec2::new(1.3, -1.3),
                    Vec2::new(-1.3, 1.3),
                ] {
                    let g_b =
                        painter.layout_no_wrap(item.ch.to_string(), item.font.clone(), P5_BLACK);
                    painter.galley(r.min + off, g_b, P5_BLACK);
                }
                let g_w = painter.layout_no_wrap(item.ch.to_string(), item.font.clone(), P5_WHITE);
                painter.galley(r.min, g_w, P5_WHITE);
            }
        }
    }

    anchor.expand2(Vec2::new(4.0, 3.0))
}

fn draw_p5_icon_badge(painter: &egui::Painter, center: Pos2, radius: f32, icon: P5Icon) {
    painter.circle_filled(center + Vec2::new(2.0, 2.2), radius + 1.5, P5_BLACK);
    painter.circle(center, radius + 1.5, P5_WHITE, Stroke::NONE);
    painter.circle_filled(center, radius, P5_BLACK);

    let ir = radius * 0.52;
    match icon {
        P5Icon::Square => {
            painter.rect_stroke(
                Rect::from_center_size(center, Vec2::splat(ir * 1.85)),
                CornerRadius::ZERO,
                Stroke::new(2.2_f32, P5_MAGENTA),
                StrokeKind::Middle,
            );
        }
        P5Icon::Triangle => {
            let pts = vec![
                Pos2::new(center.x, center.y - ir * 1.05),
                Pos2::new(center.x + ir * 1.0, center.y + ir * 0.85),
                Pos2::new(center.x - ir * 1.0, center.y + ir * 0.85),
            ];
            painter.add(Shape::closed_line(pts, Stroke::new(2.2_f32, P5_CYAN)));
        }
        P5Icon::Circle => {
            painter.circle_stroke(center, ir * 0.95, Stroke::new(2.2_f32, P5_RED));
        }
        P5Icon::Cross => {
            let d = ir * 0.85;
            painter.line_segment(
                [Pos2::new(center.x - d, center.y - d), Pos2::new(center.x + d, center.y + d)],
                Stroke::new(2.2_f32, P5_GOLD),
            );
            painter.line_segment(
                [Pos2::new(center.x + d, center.y - d), Pos2::new(center.x - d, center.y + d)],
                Stroke::new(2.2_f32, P5_GOLD),
            );
        }
    }
}

pub fn create(
    params: Arc<PhantomParams>,
    ui_state: Arc<SharedUiState>,
    updater: UpdaterHandle,
) -> Option<Box<dyn Editor>> {
    let egui_state = params.editor_state.clone();
    create_egui_editor(
        egui_state,
        EditorUiState::default(),
        |egui_ctx, _| {
            setup_p5_fonts(egui_ctx);

            let mut style = (*egui_ctx.style()).clone();
            style.visuals = egui::Visuals::dark();
            style.visuals.panel_fill = P5_BG;
            style.visuals.window_fill = P5_BG;
            style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(26, 26, 33);
            style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, P5_WHITE);
            style.visuals.widgets.hovered.bg_fill = P5_RED_DEEP;
            style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.5_f32, P5_WHITE);
            style.visuals.widgets.active.bg_fill = P5_RED;
            style.visuals.selection.bg_fill = P5_RED;
            style.visuals.selection.stroke = Stroke::new(1.0_f32, P5_WHITE);
            egui_ctx.set_style(style);
        },
        move |egui_ctx, setter, state| {
            let (in_mask, out_mask) = ui_state.get_active_masks();
            let up_status = updater.status();
            if in_mask != 0
                || out_mask != 0
                || state.auditioning_note.is_some()
                || matches!(
                    up_status,
                    UpdateStatus::Checking | UpdateStatus::Installing(_)
                )
            {
                egui_ctx.request_repaint();
            }

            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(P5_BG).inner_margin(8.0))
                .show(egui_ctx, |ui| {
                    let full_rect = ui.available_rect_before_wrap();
                    let piano_h = 104.0;
                    let top_h = (full_rect.height() - piano_h - 8.0).max(320.0);

                    ui.allocate_ui_with_layout(
                        Vec2::new(full_rect.width(), top_h),
                        egui::Layout::left_to_right(egui::Align::Min),
                        |ui| {
                            let left_w = 290.0;
                            let right_w = (full_rect.width() - left_w - 8.0).max(400.0);

                            draw_crimson_command_zone(
                                ui,
                                left_w,
                                top_h,
                                &params,
                                &updater,
                                &up_status,
                                setter,
                            );
                            ui.add_space(8.0);
                            draw_starburst_zone(
                                ui,
                                right_w,
                                top_h,
                                &params,
                                &ui_state,
                                setter,
                                state,
                            );
                        },
                    );

                    ui.add_space(8.0);
                    draw_piano_monitor(ui, in_mask, out_mask, &ui_state, state);
                });
        },
    )
}

fn draw_crimson_command_zone(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    params: &PhantomParams,
    updater: &UpdaterHandle,
    up_status: &UpdateStatus,
    setter: &ParamSetter,
) {
    let (zone_rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter_at(zone_rect);

    painter.rect_filled(zone_rect, CornerRadius::same(4), P5_RED_DEEP);
    let inner_crimson = Rect::from_min_max(
        Pos2::new(zone_rect.left(), zone_rect.top()),
        Pos2::new(zone_rect.right() - 28.0, zone_rect.bottom() - 24.0),
    );
    painter.rect_filled(inner_crimson, CornerRadius::ZERO, P5_RED);

    let slash = vec![
        Pos2::new(zone_rect.right() - 75.0, zone_rect.bottom()),
        Pos2::new(zone_rect.right(), zone_rect.bottom() - 120.0),
        Pos2::new(zone_rect.right(), zone_rect.bottom()),
    ];
    painter.add(Shape::convex_polygon(slash, P5_BG, Stroke::NONE));

    for row in 0..9 {
        for col in 0..9 {
            let r = 3.2 - (row as f32 + col as f32) * 0.17;
            if r > 0.6 {
                let cx = zone_rect.right() - 12.0 - col as f32 * 11.0;
                let cy = zone_rect.top() + 12.0 + row as f32 * 11.0;
                painter.circle_filled(
                    Pos2::new(cx, cy),
                    r,
                    Color32::from_rgba_unmultiplied(255, 255, 255, 38),
                );
            }
        }
    }

    painter.rect_stroke(
        zone_rect,
        CornerRadius::same(4),
        Stroke::new(2.5_f32, P5_BLACK),
        StrokeKind::Inside,
    );

    draw_ransom_logo(&painter, Pos2::new(zone_rect.left() + 10.0, zone_rect.top() + 10.0));

    let content_rect = zone_rect.shrink2(Vec2::new(12.0, 12.0));
    let mut child_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(content_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );

    child_ui.add_space(54.0);
    let (hdr_r, _) =
        child_ui.allocate_exact_size(Vec2::new(content_rect.width(), 28.0), Sense::hover());
    draw_p5_3d_text(
        child_ui.painter(),
        Pos2::new(hdr_r.left() + 4.0, hdr_r.center().y),
        egui::Align2::LEFT_CENTER,
        "Sound Command",
        16.0,
        P5_WHITE,
        P5_WHITE,
        FONT_IMPACT,
    );
    child_ui.add_space(4.0);

    let patches = [
        (SoundPatch::VelvetRhodes, "Scarbee Pan Cake", "EP", P5Icon::Square),
        (SoundPatch::SlapBass, "Clean Fender Bass", "JB", P5Icon::Triangle),
        (SoundPatch::PhantomBrass, "Phantom Brass", "HRN", P5Icon::Circle),
        (SoundPatch::WahClav, "Wah Clav", "FNK", P5Icon::Cross),
    ];
    let active_patch = params.sound_patch.value();

    for (idx, (patch, title, tag, icon)) in patches.iter().enumerate() {
        let selected = active_patch == *patch;
        let tilt_offset: f32 = if idx % 2 == 0 { -2.0_f32 } else { 2.5_f32 };
        let row_h = 49.0;
        let (btn_rect, resp) =
            child_ui.allocate_exact_size(Vec2::new(content_rect.width(), row_h), Sense::click());

        if resp.clicked() {
            setter.begin_set_parameter(&params.sound_patch);
            setter.set_parameter(&params.sound_patch, *patch);
            setter.end_set_parameter(&params.sound_patch);
        }

        let p = child_ui.painter();
        let pill = Rect::from_min_max(
            Pos2::new(btn_rect.left() + 2.0 + tilt_offset.max(0.0), btn_rect.top() + 3.0),
            Pos2::new(btn_rect.right() - 4.0 + tilt_offset.min(0.0), btn_rect.bottom() - 3.0),
        );

        if selected {
            p.rect_filled(
                pill.translate(Vec2::new(4.0, 4.0)),
                CornerRadius::same(2),
                P5_BLACK,
            );
            p.rect(
                pill,
                CornerRadius::same(2),
                P5_WHITE,
                Stroke::new(2.5_f32, P5_BLACK),
                StrokeKind::Inside,
            );
        } else if resp.hovered() {
            p.rect_filled(
                pill.translate(Vec2::new(3.0, 3.0)),
                CornerRadius::same(2),
                P5_BLACK,
            );
            p.rect(
                pill,
                CornerRadius::same(2),
                Color32::from_rgb(48, 8, 16),
                Stroke::new(1.5_f32, P5_WHITE),
                StrokeKind::Inside,
            );
        }

        draw_p5_icon_badge(p, Pos2::new(pill.left() + 20.0, pill.center().y), 12.0, *icon);

        let rim = if selected { P5_RED } else { P5_WHITE };
        draw_p5_3d_text(
            p,
            Pos2::new(pill.left() + 42.0, pill.center().y),
            egui::Align2::LEFT_CENTER,
            title,
            17.5,
            P5_WHITE,
            rim,
            FONT_IMPACT,
        );

        if selected {
            let cy = pill.center().y;
            let rx = pill.right() - 8.0;
            let wedge = vec![
                Pos2::new(rx - 14.0, cy - 8.5),
                Pos2::new(rx, cy),
                Pos2::new(rx - 14.0, cy + 8.5),
            ];
            p.add(Shape::convex_polygon(
                wedge,
                P5_CYAN,
                Stroke::new(1.5_f32, P5_BLACK),
            ));
        } else {
            draw_p5_3d_text(
                p,
                Pos2::new(pill.right() - 8.0, pill.center().y),
                egui::Align2::RIGHT_CENTER,
                tag,
                12.0,
                P5_GOLD,
                P5_WHITE,
                FONT_IMPACT,
            );
        }

        child_ui.add_space(3.0);
    }

    child_ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ui.horizontal(|ui| {
            let audio_on = params.preview_enabled.value();
            let (ab_rect, ab_resp) = ui.allocate_exact_size(Vec2::new(86.0, 30.0), Sense::click());
            if ab_resp.clicked() {
                setter.begin_set_parameter(&params.preview_enabled);
                setter.set_parameter(&params.preview_enabled, !audio_on);
                setter.end_set_parameter(&params.preview_enabled);
            }
            draw_p5_3d_text(
                ui.painter(),
                Pos2::new(ab_rect.left() + 4.0, ab_rect.center().y),
                egui::Align2::LEFT_CENTER,
                if audio_on { "Audio On" } else { "Muted" },
                14.5,
                if audio_on { P5_CYAN } else { P5_RED },
                P5_WHITE,
                FONT_IMPACT,
            );

            // Auto-updater pill in bottom bar
            #[derive(Clone, Copy)]
            enum UpAction {
                None,
                Recheck,
                Install,
                Cleanse,
            }

            let (up_label, up_fg, up_rim, action, hover_tip): (
                String,
                Color32,
                Color32,
                UpAction,
                Option<String>,
            ) = match &up_status {
                UpdateStatus::Idle => (
                    format!("v{CURRENT_VERSION}"),
                    P5_WHITE,
                    P5_WHITE,
                    UpAction::Recheck,
                    Some("Click to check for updates & old copies".to_string()),
                ),
                UpdateStatus::Checking => (
                    "Checking...".to_string(),
                    P5_MUTED,
                    P5_WHITE,
                    UpAction::None,
                    None,
                ),
                UpdateStatus::UpToDate { stale_copies } if *stale_copies > 0 => (
                    format!("CLEANSE ({stale_copies} OLD)!"),
                    P5_GOLD,
                    P5_RED,
                    UpAction::Cleanse,
                    Some(format!(
                        "Found {stale_copies} old/duplicate plugin copy(s) — click to cleanse!"
                    )),
                ),
                UpdateStatus::UpToDate { .. } => (
                    format!("v{CURRENT_VERSION}"),
                    P5_WHITE,
                    P5_WHITE,
                    UpAction::Recheck,
                    Some("Up to date — click to check again".to_string()),
                ),
                UpdateStatus::Available {
                    version,
                    stale_copies,
                    ..
                } => (
                    if *stale_copies > 0 {
                        format!("UPDATE v{version} + CLEAN!")
                    } else {
                        format!("UPDATE v{version}!")
                    },
                    P5_GOLD,
                    P5_GOLD,
                    UpAction::Install,
                    Some("Click to update & cleanse old versions".to_string()),
                ),
                UpdateStatus::Installing(v) => (
                    format!("Working ({v})..."),
                    P5_CYAN,
                    P5_CYAN,
                    UpAction::None,
                    None,
                ),
                UpdateStatus::ReadyToRestart(msg) => (
                    msg.clone(),
                    P5_CYAN,
                    P5_CYAN,
                    UpAction::None,
                    Some("Finish in the updater popup & reopen FL Studio".to_string()),
                ),
                UpdateStatus::Error(err) => (
                    format!("v{CURRENT_VERSION} (Retry)"),
                    P5_MUTED,
                    P5_WHITE,
                    UpAction::Recheck,
                    Some(err.clone()),
                ),
            };

            let (up_rect, up_resp) = ui.allocate_exact_size(Vec2::new(142.0, 28.0), Sense::click());
            if let Some(tip) = hover_tip {
                up_resp.clone().on_hover_text(tip);
            }
            if up_resp.clicked() {
                match action {
                    UpAction::Install => {
                        if let UpdateStatus::Available {
                            version, zip_url, ..
                        } = &up_status
                        {
                            updater.install_async(version.clone(), zip_url.clone());
                        }
                    }
                    UpAction::Cleanse => updater.cleanse_async(),
                    UpAction::Recheck => updater.check_async(),
                    UpAction::None => {}
                }
            }
            draw_p5_3d_text(
                ui.painter(),
                Pos2::new(up_rect.left() + 2.0, up_rect.center().y),
                egui::Align2::LEFT_CENTER,
                &up_label,
                11.5,
                up_fg,
                if up_resp.hovered() { P5_RED } else { up_rim },
                FONT_IMPACT,
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (bar_r, _) = ui.allocate_exact_size(Vec2::new(38.0, 14.0), Sense::hover());
                let bp = ui.painter_at(bar_r);
                bp.rect_filled(
                    Rect::from_min_size(bar_r.min, Vec2::new(16.0, 5.0)),
                    CornerRadius::ZERO,
                    P5_CYAN,
                );
                bp.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(bar_r.min.x + 19.0, bar_r.min.y),
                        Vec2::new(16.0, 5.0),
                    ),
                    CornerRadius::ZERO,
                    P5_MAGENTA,
                );
            });
        });
    });
}

fn draw_ransom_logo(painter: &egui::Painter, origin: Pos2) {
    let strip = Rect::from_min_size(origin, Vec2::new(268.0, 42.0));
    painter.rect_filled(strip.translate(Vec2::new(3.5, 3.5)), CornerRadius::ZERO, P5_BLACK);
    painter.rect(
        strip,
        CornerRadius::ZERO,
        P5_WHITE,
        Stroke::new(2.5_f32, P5_BLACK),
        StrokeKind::Inside,
    );

    let tiles = [
        ("P", P5_BLACK, P5_WHITE, -2.5_f32, FONT_HEAVY),
        ("H", P5_WHITE, P5_RED, 2.0_f32, FONT_SERIF),
        ("a", P5_RED, P5_BLACK, -1.5_f32, FONT_SERIF),
        ("N", P5_BLACK, P5_WHITE, 2.5_f32, FONT_IMPACT),
        ("t", P5_WHITE, P5_BLACK, -2.0_f32, FONT_SERIF),
        ("o", P5_RED, P5_WHITE, 1.5_f32, FONT_HEAVY),
        ("M", P5_BLACK, P5_WHITE, -1.5_f32, FONT_IMPACT),
    ];

    let mut x = origin.x + 6.0;
    for (ch, bg, fg, dy, fam) in tiles {
        let tr = Rect::from_min_size(Pos2::new(x, origin.y + 7.0 + dy), Vec2::new(23.0, 27.0));
        painter.rect_filled(tr.translate(Vec2::new(2.0, 2.0)), CornerRadius::ZERO, P5_BLACK);
        painter.rect(
            tr,
            CornerRadius::ZERO,
            bg,
            Stroke::new(1.8_f32, P5_BLACK),
            StrokeKind::Inside,
        );
        painter.text(
            tr.center(),
            egui::Align2::CENTER_CENTER,
            ch,
            FontId::new(18.0, FontFamily::Name(fam.into())),
            fg,
        );
        x += 24.5;
    }

    draw_p5_3d_text(
        painter,
        Pos2::new(x + 4.0, origin.y + 21.0),
        egui::Align2::LEFT_CENTER,
        "VOICER",
        16.5,
        P5_WHITE,
        P5_RED,
        FONT_IMPACT,
    );
}

fn draw_starburst_zone(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    params: &PhantomParams,
    ui_state: &SharedUiState,
    setter: &ParamSetter,
    state: &mut EditorUiState,
) {
    let (zone_rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter_at(zone_rect);

    painter.rect_filled(zone_rect, CornerRadius::same(4), Color32::from_rgb(12, 12, 16));

    let step = 86.0;
    let cols = (zone_rect.width() / step).ceil() as i32 + 1;
    let rows = (zone_rect.height() / step).ceil() as i32 + 1;
    for r in 0..rows {
        for c in 0..cols {
            let offset_x = if r % 2 == 0 { 0.0 } else { step * 0.5 };
            let center = Pos2::new(
                zone_rect.left() + c as f32 * step + offset_x,
                zone_rect.top() + r as f32 * step,
            );
            draw_star_outline(
                &painter,
                center,
                34.0,
                14.0,
                0.25,
                Stroke::new(2.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 28)),
            );
            draw_star_outline(
                &painter,
                center,
                18.0,
                7.5,
                0.25,
                Stroke::new(1.5_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 16)),
            );
        }
    }

    painter.rect_stroke(
        zone_rect,
        CornerRadius::same(4),
        Stroke::new(2.5_f32, P5_BLACK),
        StrokeKind::Inside,
    );

    let content_rect = zone_rect.shrink(12.0);
    let mut child_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(content_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );

    let chord_assist = params.chord_assist.value();
    let header_h = 62.0;
    let (hdr_rect, _) =
        child_ui.allocate_exact_size(Vec2::new(content_rect.width(), header_h), Sense::hover());

    let mut hdr_left = child_ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(
                hdr_rect.min,
                Pos2::new(hdr_rect.right() - 190.0, hdr_rect.bottom()),
            ))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );

    hdr_left.horizontal(|ui| {
        let (t1_r, t1_resp) = ui.allocate_exact_size(Vec2::new(112.0, 30.0), Sense::click());
        if t1_resp.clicked() {
            state.active_tab = UiTab::MainRack;
        }
        draw_p5_3d_text(
            ui.painter(),
            t1_r.center(),
            egui::Align2::CENTER_CENTER,
            "Main Rack",
            14.0,
            if state.active_tab == UiTab::MainRack { P5_CYAN } else { P5_WHITE },
            if state.active_tab == UiTab::MainRack { P5_CYAN } else { P5_WHITE },
            FONT_IMPACT,
        );

        let (t2_r, t2_resp) = ui.allocate_exact_size(Vec2::new(124.0, 30.0), Sense::click());
        if t2_resp.clicked() {
            state.active_tab = UiTab::Mark1Kore;
            if params.sound_patch.value() != SoundPatch::VelvetRhodes {
                setter.begin_set_parameter(&params.sound_patch);
                setter.set_parameter(&params.sound_patch, SoundPatch::VelvetRhodes);
                setter.end_set_parameter(&params.sound_patch);
            }
        }
        draw_p5_3d_text(
            ui.painter(),
            t2_r.center(),
            egui::Align2::CENTER_CENTER,
            "Mark I Kore",
            14.0,
            if state.active_tab == UiTab::Mark1Kore { P5_GOLD } else { P5_WHITE },
            if state.active_tab == UiTab::Mark1Kore { P5_GOLD } else { P5_WHITE },
            FONT_IMPACT,
        );
    });

    hdr_left.add_space(2.0);
    hdr_left.horizontal(|ui| {
        let mode_btn = egui::Button::new(
            RichText::new(if chord_assist {
                "MODE: CHORD ASSIST"
            } else {
                "MODE: SINGLE NOTE"
            })
            .size(10.5)
            .strong()
            .color(if chord_assist { P5_BLACK } else { P5_WHITE }),
        )
        .fill(if chord_assist { P5_GOLD } else { P5_BLACK })
        .stroke(Stroke::new(1.5_f32, P5_WHITE));

        if ui.add(mode_btn).clicked() {
            setter.begin_set_parameter(&params.chord_assist);
            setter.set_parameter(&params.chord_assist, !chord_assist);
            setter.end_set_parameter(&params.chord_assist);
        }
        ui.label(
            RichText::new(if chord_assist {
                "Keys trigger P5 chords"
            } else {
                "Play melodies & chords freely"
            })
            .size(10.5)
            .color(P5_MUTED),
        );
    });

    // Trapezoid Active Note / Chord Callout on right
    let trap_rect = Rect::from_min_max(
        Pos2::new(hdr_rect.right() - 182.0, hdr_rect.top()),
        Pos2::new(hdr_rect.right(), hdr_rect.bottom() - 2.0),
    );
    let hp = child_ui.painter();
    let trap_shadow = vec![
        Pos2::new(trap_rect.left() + 14.0, trap_rect.top() + 5.0),
        Pos2::new(trap_rect.right() + 5.0, trap_rect.top() + 5.0),
        Pos2::new(trap_rect.right() - 6.0, trap_rect.bottom() + 5.0),
        Pos2::new(trap_rect.left() + 4.0, trap_rect.bottom() + 5.0),
    ];
    hp.add(Shape::convex_polygon(trap_shadow, P5_RED, Stroke::NONE));

    let trap_pts = vec![
        Pos2::new(trap_rect.left() + 10.0, trap_rect.top()),
        Pos2::new(trap_rect.right(), trap_rect.top()),
        Pos2::new(trap_rect.right() - 10.0, trap_rect.bottom()),
        Pos2::new(trap_rect.left(), trap_rect.bottom()),
    ];
    hp.add(Shape::convex_polygon(
        trap_pts,
        P5_WHITE,
        Stroke::new(2.5_f32, P5_BLACK),
    ));

    hp.text(
        Pos2::new(trap_rect.center().x, trap_rect.top() + 5.0),
        egui::Align2::CENTER_TOP,
        if chord_assist { "ACTIVE CHORD" } else { "ACTIVE NOTE" },
        FontId::new(10.0, FontFamily::Name(FONT_HEAVY.into())),
        P5_RED,
    );
    let readout = ui_state.get_chord_name();
    draw_p5_3d_text(
        hp,
        Pos2::new(trap_rect.center().x, trap_rect.bottom() - 18.0),
        egui::Align2::CENTER_CENTER,
        &readout,
        22.0,
        P5_WHITE,
        P5_RED,
        FONT_IMPACT,
    );

    child_ui.add_space(8.0);

    if state.active_tab == UiTab::Mark1Kore {
        draw_mark1_kore_rack(&mut child_ui, params, setter);
        return;
    }

    child_ui.columns(2, |cols| {
        draw_cutout_card(&mut cols[0], "01 // Tone & Env", P5_GOLD, |ui| {
            ui.horizontal(|ui| {
                let kw = (ui.available_width() - 12.0) / 2.0;
                p5_knob(ui, "Bright", &params.brightness, setter, P5_RED, kw);
                p5_knob(ui, "Glide", &params.glide_ms, setter, P5_CYAN, kw);
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let kw = (ui.available_width() - 12.0) / 2.0;
                p5_knob(ui, "Attack", &params.attack_ms, setter, P5_GOLD, kw);
                p5_knob(ui, "Release", &params.release_ms, setter, P5_WHITE, kw);
            });
        });

        draw_cutout_card(&mut cols[1], "02 // P5 FX Rack", P5_RED, |ui| {
            ui.horizontal(|ui| {
                let kw = (ui.available_width() - 12.0) / 2.0;
                p5_knob(ui, "Bark", &params.rhodes_bark, setter, P5_RED, kw);
                p5_knob(ui, "Auto-Wah", &params.wah_amount, setter, P5_GOLD, kw);
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let kw = (ui.available_width() - 12.0) / 2.0;
                p5_knob(ui, "Pan Depth", &params.tremolo_depth, setter, P5_CYAN, kw);
                p5_knob(ui, "Pan Rate", &params.tremolo_rate_hz, setter, P5_WHITE, kw);
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Out Gain").size(10.5).strong().color(P5_MUTED));
                ui.add(
                    nih_plug_egui::widgets::ParamSlider::for_param(&params.preview_gain, setter)
                        .with_width(ui.available_width()),
                );
            });
        });
    });

    child_ui.add_space(8.0);

    let card3_border = if chord_assist { P5_GOLD } else { Color32::from_rgb(72, 72, 84) };
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(15, 15, 20, 240))
        .stroke(Stroke::new(2.0_f32, card3_border))
        .inner_margin(8.0)
        .show(&mut child_ui, |ui| {
            let (t_rect, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
            draw_p5_3d_text(
                ui.painter(),
                Pos2::new(t_rect.left() + 4.0, t_rect.center().y),
                egui::Align2::LEFT_CENTER,
                "03 // Chord Assist",
                14.5,
                if chord_assist { P5_GOLD } else { P5_WHITE },
                if chord_assist { P5_GOLD } else { P5_WHITE },
                FONT_IMPACT,
            );
            ui.painter().text(
                Pos2::new(t_rect.left() + 148.0, t_rect.center().y),
                egui::Align2::LEFT_CENTER,
                if chord_assist { "(ACTIVE)" } else { "(OFF — OPTIONAL)" },
                FontId::proportional(10.5),
                P5_MUTED,
            );

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                enum_combo_compact(
                    ui,
                    "Key",
                    &params.key_root,
                    setter,
                    &[
                        KeyRoot::C,
                        KeyRoot::Cs,
                        KeyRoot::D,
                        KeyRoot::Eb,
                        KeyRoot::E,
                        KeyRoot::F,
                        KeyRoot::Fs,
                        KeyRoot::G,
                        KeyRoot::Ab,
                        KeyRoot::A,
                        KeyRoot::Bb,
                        KeyRoot::B,
                    ],
                    64.0,
                );
                enum_combo_compact(
                    ui,
                    "Scale",
                    &params.scale_mode,
                    setter,
                    &[
                        ScaleMode::Dorian,
                        ScaleMode::HarmonicMinor,
                        ScaleMode::NaturalMinor,
                        ScaleMode::Major,
                        ScaleMode::Chromatic,
                    ],
                    148.0,
                );
                enum_combo_compact(
                    ui,
                    "Style",
                    &params.voicing_style,
                    setter,
                    &[
                        VoicingStyle::AcidJazz,
                        VoicingStyle::NeoSoul,
                        VoicingStyle::FunkStab,
                        VoicingStyle::DimPassing,
                    ],
                    152.0,
                );
                enum_combo_compact(
                    ui,
                    "Strum",
                    &params.strum_dir,
                    setter,
                    &[
                        StrumDir::Up,
                        StrumDir::Down,
                        StrumDir::Alternate,
                        StrumDir::Tight,
                    ],
                    84.0,
                );
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                bool_toggle(ui, "Voice Lead", &params.voice_lead, setter);
                bool_toggle(ui, "Bass Split", &params.bass_split, setter);
                ui.add_space(6.0);
                ui.label(RichText::new("Strum ms").size(11.0).color(P5_MUTED));
                ui.add(
                    nih_plug_egui::widgets::ParamSlider::for_param(&params.strum_ms, setter)
                        .with_width(100.0),
                );
                ui.label(RichText::new("Ext").size(11.0).color(P5_MUTED));
                ui.add(
                    nih_plug_egui::widgets::ParamSlider::for_param(&params.extensions, setter)
                        .with_width(92.0),
                );
            });
        });
}

fn draw_mark1_kore_rack(ui: &mut egui::Ui, params: &PhantomParams, setter: &ParamSetter) {
    // =========================================================================
    // TOP SECTION: Persona 5 Calling Card "SCARBEE MARK I" Tone & Noise Deck
    // =========================================================================
    let top_w = ui.available_width();
    let top_h = 118.0;
    let (top_rect, _) = ui.allocate_exact_size(Vec2::new(top_w, top_h), Sense::hover());
    {
        let tp = ui.painter_at(top_rect);
        // Hard comic shadow + dark matte card with crimson angled slash + halftone dots
        tp.rect_filled(top_rect.translate(Vec2::new(3.0, 3.0)), CornerRadius::same(3), P5_BLACK);
        tp.rect_filled(top_rect, CornerRadius::same(3), Color32::from_rgb(16, 16, 22));

        let mid_slash = vec![
            Pos2::new(top_rect.center().x - 95.0, top_rect.top()),
            Pos2::new(top_rect.center().x + 105.0, top_rect.top()),
            Pos2::new(top_rect.center().x + 78.0, top_rect.bottom()),
            Pos2::new(top_rect.center().x - 122.0, top_rect.bottom()),
        ];
        tp.add(Shape::convex_polygon(mid_slash, P5_RED, Stroke::NONE));

        for row in 0..6 {
            for col in 0..8 {
                let r = 2.6 - (row as f32 * 0.25);
                if r > 0.6 {
                    tp.circle_filled(
                        Pos2::new(
                            top_rect.center().x - 72.0 + col as f32 * 20.0,
                            top_rect.top() + 10.0 + row as f32 * 18.0,
                        ),
                        r,
                        Color32::from_rgba_unmultiplied(255, 255, 255, 34),
                    );
                }
            }
        }
        tp.rect_stroke(
            top_rect,
            CornerRadius::same(3),
            Stroke::new(2.0_f32, P5_WHITE),
            StrokeKind::Inside,
        );
    }

    let mut top_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(top_rect.shrink2(Vec2::new(10.0, 8.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    top_ui.spacing_mut().item_spacing.x = 4.0;

    let top_inner_w = (top_rect.width() - 20.0).max(460.0);
    let center_w = 122.0_f32;
    let col_w = ((top_inner_w - center_w - 16.0) / 4.0).max(78.0);
    mk1_p5_knob(&mut top_ui, "BASS", &params.mk1_bass, setter, P5_CYAN, col_w);
    mk1_p5_knob(&mut top_ui, "TREBLE", &params.mk1_treble, setter, P5_GOLD, col_w);

    // Center Scarbee Mark I P5 Menu-Font Emblem
    top_ui.allocate_ui_with_layout(
        Vec2::new(center_w, 98.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            let (r1, _) = ui.allocate_exact_size(Vec2::new(col_w + 14.0, 32.0), Sense::hover());
            draw_p5_ransom_word(
                ui.painter(),
                r1.center(),
                egui::Align2::CENTER_CENTER,
                "SCARBEE",
                14.5,
                P5_GOLD,
            );
            let (r2, _) = ui.allocate_exact_size(Vec2::new(col_w + 14.0, 34.0), Sense::hover());
            draw_p5_ransom_word(
                ui.painter(),
                r2.center(),
                egui::Align2::CENTER_CENTER,
                "MARK I",
                18.0,
                P5_RED,
            );
            let (r3, _) = ui.allocate_exact_size(Vec2::new(col_w + 14.0, 22.0), Sense::hover());
            draw_p5_3d_text(
                ui.painter(),
                r3.center(),
                egui::Align2::CENTER_CENTER,
                "Kore Edition",
                11.0,
                P5_GOLD,
                P5_WHITE,
                FONT_IMPACT,
            );
        },
    );

    mk1_p5_knob(
        &mut top_ui,
        "STRIKE NOISE",
        &params.mk1_noise_level,
        setter,
        P5_WHITE,
        col_w,
    );
    mk1_p5_knob(
        &mut top_ui,
        "KEY RELEASE",
        &params.mk1_release_noise,
        setter,
        P5_RED,
        col_w,
    );

    ui.add_space(8.0);

    // =========================================================================
    // BOTTOM SECTION: Persona 5 Insert FX Matrix ("Pan Cake Kore")
    // =========================================================================
    let bot_w = ui.available_width();
    let bot_h = 194.0;
    let (bot_rect, _) = ui.allocate_exact_size(Vec2::new(bot_w, bot_h), Sense::hover());
    {
        let bp = ui.painter_at(bot_rect);
        bp.rect_filled(bot_rect.translate(Vec2::new(3.0, 3.0)), CornerRadius::same(3), P5_BLACK);
        bp.rect_filled(bot_rect, CornerRadius::same(3), Color32::from_rgb(14, 14, 19));

        // Subtle diagonal crimson accent bars in corners
        let corner_tl = vec![
            bot_rect.min,
            Pos2::new(bot_rect.left() + 68.0, bot_rect.top()),
            Pos2::new(bot_rect.left(), bot_rect.top() + 42.0),
        ];
        bp.add(Shape::convex_polygon(corner_tl, P5_RED, Stroke::NONE));

        let corner_br = vec![
            bot_rect.max,
            Pos2::new(bot_rect.right() - 78.0, bot_rect.bottom()),
            Pos2::new(bot_rect.right(), bot_rect.bottom() - 46.0),
        ];
        bp.add(Shape::convex_polygon(corner_br, P5_RED_DEEP, Stroke::NONE));

        bp.rect_stroke(
            bot_rect,
            CornerRadius::same(3),
            Stroke::new(2.0_f32, P5_WHITE),
            StrokeKind::Inside,
        );
    }

    let mut bot_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bot_rect.shrink2(Vec2::new(10.0, 8.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Min)),
    );
    bot_ui.spacing_mut().item_spacing.x = 4.0;

    let bot_inner_w = (bot_rect.width() - 20.0).max(460.0);
    let pill_col_w = if bot_inner_w < 540.0 { 82.0 } else { 94.0 };
    let center_col_w = if bot_inner_w < 540.0 { 102.0 } else { 116.0 };
    let knob_col_w = ((bot_inner_w - pill_col_w * 2.0 - center_col_w - 16.0) / 2.0).max(96.0);
    let knob_w = ((knob_col_w - 4.0) / 2.0).floor();

    // Left FX Command Pills (PAN, AUTOWAH, COMP., CHORUS)
    bot_ui.allocate_ui_with_layout(
        Vec2::new(pill_col_w, 178.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.add_space(4.0);
            mk1_p5_fx_pill(ui, "PAN", &params.mk1_fx_pan, setter, P5_CYAN);
            ui.add_space(6.0);
            mk1_p5_fx_pill(ui, "AUTOWAH", &params.mk1_fx_autowah, setter, P5_GOLD);
            ui.add_space(6.0);
            mk1_p5_fx_pill(ui, "COMP.", &params.mk1_fx_comp, setter, P5_WHITE);
            ui.add_space(6.0);
            mk1_p5_fx_pill(ui, "CHORUS", &params.mk1_fx_chorus, setter, P5_RED);
        },
    );

    // Left-Center Pan Controls (Intensity, Speed, Tempo Sync)
    bot_ui.allocate_ui_with_layout(
        Vec2::new(knob_col_w, 178.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            let (hdr_r, _) = ui.allocate_exact_size(Vec2::new(knob_col_w - 4.0, 24.0), Sense::hover());
            draw_p5_3d_text(
                ui.painter(),
                hdr_r.center(),
                egui::Align2::CENTER_CENTER,
                "Pan Cake",
                13.0,
                P5_CYAN,
                P5_WHITE,
                FONT_IMPACT,
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                mk1_p5_knob(ui, "INTENSITY", &params.mk1_pan_intensity, setter, P5_CYAN, knob_w);
                mk1_p5_knob(ui, "SPEED", &params.mk1_pan_speed, setter, P5_GOLD, knob_w);
            });
            ui.add_space(4.0);
            let sync = params.mk1_pan_sync.value();
            let (sr, sresp) = ui.allocate_exact_size(Vec2::new(96.0, 26.0), Sense::click());
            if sresp.clicked() {
                setter.begin_set_parameter(&params.mk1_pan_sync);
                setter.set_parameter(&params.mk1_pan_sync, !sync);
                setter.end_set_parameter(&params.mk1_pan_sync);
            }
            draw_p5_3d_text(
                ui.painter(),
                sr.center(),
                egui::Align2::CENTER_CENTER,
                if sync { "Sync: DAW" } else { "Sync: Free" },
                12.0,
                if sync { P5_GOLD } else { P5_MUTED },
                if sresp.hovered() { P5_RED } else { P5_WHITE },
                FONT_IMPACT,
            );
        },
    );

    // Center Preset & Insert Badge ("Pan Cake Kore" Ransom Button)
    bot_ui.allocate_ui_with_layout(
        Vec2::new(center_col_w, 178.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.add_space(6.0);
            let (ir, _) = ui.allocate_exact_size(Vec2::new(center_col_w - 4.0, 24.0), Sense::hover());
            draw_p5_3d_text(
                ui.painter(),
                ir.center(),
                egui::Align2::CENTER_CENTER,
                "Insert FX",
                12.5,
                P5_WHITE,
                P5_RED,
                FONT_IMPACT,
            );
            ui.add_space(10.0);
            ui.label(RichText::new("EFFECT PRESET").size(9.5).strong().color(P5_MUTED));
            ui.add_space(4.0);

            let (pr, presp) = ui.allocate_exact_size(Vec2::new(center_col_w - 4.0, 56.0), Sense::click());
            let pp = ui.painter();
            let card = pr.shrink2(Vec2::new(2.0, 4.0));
            pp.rect_filled(card.translate(Vec2::new(3.0, 3.0)), CornerRadius::ZERO, P5_BLACK);
            pp.rect(
                card,
                CornerRadius::ZERO,
                if presp.hovered() { P5_RED } else { Color32::from_rgb(24, 24, 30) },
                Stroke::new(2.0_f32, P5_GOLD),
                StrokeKind::Inside,
            );
            draw_p5_ransom_word(
                pp,
                Pos2::new(card.center().x, card.center().y - 9.0),
                egui::Align2::CENTER_CENTER,
                "PAN CAKE",
                12.5,
                P5_GOLD,
            );
            draw_p5_3d_text(
                pp,
                Pos2::new(card.center().x, card.center().y + 13.0),
                egui::Align2::CENTER_CENTER,
                "Restore Kore",
                10.5,
                P5_WHITE,
                P5_WHITE,
                FONT_IMPACT,
            );
            if presp
                .on_hover_text("Click to restore exact Scarbee Mark I Pan Cake Kore defaults")
                .clicked()
            {
                reset_pancake_kore_preset(params, setter);
            }
        },
    );

    // Right-Center Amp Controls (Drive, Output)
    bot_ui.allocate_ui_with_layout(
        Vec2::new(knob_col_w, 178.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            let (hdr_r, _) = ui.allocate_exact_size(Vec2::new(knob_col_w - 4.0, 24.0), Sense::hover());
            draw_p5_3d_text(
                ui.painter(),
                hdr_r.center(),
                egui::Align2::CENTER_CENTER,
                "Tube Amp",
                13.0,
                P5_RED,
                P5_WHITE,
                FONT_IMPACT,
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                mk1_p5_knob(ui, "DRIVE", &params.mk1_amp_drive, setter, P5_RED, knob_w);
                mk1_p5_knob(ui, "OUTPUT", &params.mk1_amp_output, setter, P5_WHITE, knob_w);
            });
            ui.add_space(6.0);
            ui.label(
                RichText::new("2x12 Cab Sim")
                    .size(10.0)
                    .strong()
                    .color(P5_MUTED),
            );
        },
    );

    // Right FX Command Pills (DIST., PHASER, AMP)
    bot_ui.allocate_ui_with_layout(
        Vec2::new(pill_col_w, 178.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.add_space(16.0);
            mk1_p5_fx_pill(ui, "DIST.", &params.mk1_fx_dist, setter, P5_RED);
            ui.add_space(10.0);
            mk1_p5_fx_pill(ui, "PHASER", &params.mk1_fx_phaser, setter, P5_MAGENTA);
            ui.add_space(10.0);
            mk1_p5_fx_pill(ui, "AMP", &params.mk1_fx_amp, setter, P5_GOLD);
        },
    );
}

fn mk1_p5_fx_pill(
    ui: &mut egui::Ui,
    label: &str,
    param: &BoolParam,
    setter: &ParamSetter,
    accent: Color32,
) {
    let active = param.value();
    let pill_w = ui.available_width().min(92.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(pill_w, 34.0), Sense::click());
    if resp.clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, !active);
        setter.end_set_parameter(param);
    }

    let p = ui.painter();
    let pill = rect.shrink2(Vec2::new(2.0, 2.0));
    p.rect_filled(pill.translate(Vec2::new(2.5, 2.5)), CornerRadius::ZERO, P5_BLACK);

    let bg = if active {
        P5_WHITE
    } else if resp.hovered() {
        Color32::from_rgb(38, 16, 22)
    } else {
        Color32::from_rgb(18, 18, 24)
    };
    let border = if active { P5_BLACK } else { Color32::from_rgb(95, 95, 110) };
    p.rect(
        pill,
        CornerRadius::ZERO,
        bg,
        Stroke::new(1.8_f32, border),
        StrokeKind::Inside,
    );

    // P5 tilted diamond LED indicator
    let c = Pos2::new(pill.left() + 12.0, pill.center().y);
    let diamond = vec![
        Pos2::new(c.x, c.y - 5.5),
        Pos2::new(c.x + 5.5, c.y),
        Pos2::new(c.x, c.y + 5.5),
        Pos2::new(c.x - 5.5, c.y),
    ];
    p.add(Shape::convex_polygon(
        diamond,
        if active { accent } else { Color32::from_rgb(48, 48, 56) },
        Stroke::new(1.2_f32, P5_BLACK),
    ));

    draw_p5_3d_text(
        p,
        Pos2::new(pill.left() + 23.0, pill.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        12.5,
        if active { P5_WHITE } else { P5_MUTED },
        if active { accent } else { P5_WHITE },
        FONT_IMPACT,
    );
}

fn mk1_p5_knob<P: Param>(
    ui: &mut egui::Ui,
    label: &str,
    param: &P,
    setter: &ParamSetter,
    accent: Color32,
    width: f32,
) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, 96.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            let knob_size = 44.0;
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::splat(knob_size), Sense::click_and_drag());

            if resp.drag_started() {
                setter.begin_set_parameter(param);
            }
            if resp.dragged() {
                let delta = -resp.drag_delta().y * 0.005;
                let cur = param.unmodulated_normalized_value();
                let next = (cur + delta).clamp(0.0, 1.0);
                setter.set_parameter(param, param.preview_plain(next));
            }
            if resp.drag_stopped() {
                setter.end_set_parameter(param);
            }
            if resp.double_clicked() {
                setter.begin_set_parameter(param);
                setter.set_parameter(param, param.default_plain_value());
                setter.end_set_parameter(param);
            }

            let norm = param.modulated_normalized_value().clamp(0.0, 1.0);
            let p = ui.painter();
            let center = rect.center();
            let r = knob_size * 0.44;

            p.circle_filled(center + Vec2::new(2.0, 2.0), r, P5_BLACK);
            p.circle_filled(center, r, Color32::from_rgb(24, 24, 30));
            p.circle_stroke(center, r, Stroke::new(1.6_f32, P5_WHITE));

            let start_ang = 0.75 * PI;
            let sweep = 1.5 * PI;
            let end_ang = start_ang + norm * sweep;

            let bg_pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let a = start_ang + (i as f32 / 24.0) * sweep;
                    Pos2::new(center.x + a.cos() * (r + 3.0), center.y + a.sin() * (r + 3.0))
                })
                .collect();
            p.add(Shape::line(
                bg_pts,
                Stroke::new(3.0_f32, Color32::from_rgb(52, 52, 64)),
            ));

            let active_steps = ((norm * 24.0).round() as usize).max(1);
            let fg_pts: Vec<Pos2> = (0..=active_steps)
                .map(|i| {
                    let a = start_ang + (i as f32 / active_steps as f32) * (norm * sweep);
                    Pos2::new(center.x + a.cos() * (r + 3.0), center.y + a.sin() * (r + 3.0))
                })
                .collect();
            p.add(Shape::line(fg_pts, Stroke::new(3.2_f32, accent)));

            let ptr_end = Pos2::new(
                center.x + end_ang.cos() * (r - 3.0),
                center.y + end_ang.sin() * (r - 3.0),
            );
            p.line_segment([center, ptr_end], Stroke::new(2.2_f32, P5_WHITE));

            let (lbl_r, _) = ui.allocate_exact_size(Vec2::new(width, 20.0), Sense::hover());
            draw_p5_3d_text(
                ui.painter(),
                lbl_r.center(),
                egui::Align2::CENTER_CENTER,
                label,
                10.5,
                P5_WHITE,
                P5_WHITE,
                FONT_IMPACT,
            );
            ui.label(
                RichText::new(param.to_string())
                    .size(10.0)
                    .strong()
                    .color(accent),
            );
        },
    );
}

fn reset_pancake_kore_preset(params: &PhantomParams, setter: &ParamSetter) {
    let set_f = |p: &FloatParam, v: f32| {
        setter.begin_set_parameter(p);
        setter.set_parameter(p, v);
        setter.end_set_parameter(p);
    };
    let set_b = |p: &BoolParam, v: bool| {
        setter.begin_set_parameter(p);
        setter.set_parameter(p, v);
        setter.end_set_parameter(p);
    };
    set_f(&params.mk1_bass, 0.05);
    set_f(&params.mk1_treble, 0.12);
    set_f(&params.mk1_noise_level, 0.45);
    set_f(&params.mk1_release_noise, 0.48);
    set_b(&params.mk1_fx_pan, true);
    set_b(&params.mk1_fx_autowah, false);
    set_b(&params.mk1_fx_comp, false);
    set_b(&params.mk1_fx_chorus, true);
    set_b(&params.mk1_fx_dist, false);
    set_b(&params.mk1_fx_phaser, false);
    set_b(&params.mk1_fx_amp, true);
    set_f(&params.mk1_pan_intensity, 0.45);
    set_f(&params.mk1_pan_speed, 0.55);
    set_b(&params.mk1_pan_sync, true);
    set_f(&params.mk1_amp_drive, 0.35);
    set_f(&params.mk1_amp_output, 0.58);
}

fn draw_cutout_card(
    ui: &mut egui::Ui,
    title: &str,
    accent: Color32,
    body: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(16, 16, 21, 242))
        .stroke(Stroke::new(2.0_f32, P5_WHITE))
        .inner_margin(9.0)
        .show(ui, |ui| {
            ui.set_min_height(214.0);
            let (tag_rect, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
            draw_p5_3d_text(
                ui.painter(),
                Pos2::new(tag_rect.left() + 4.0, tag_rect.center().y),
                egui::Align2::LEFT_CENTER,
                title,
                14.5,
                accent,
                P5_WHITE,
                FONT_IMPACT,
            );
            ui.add_space(6.0);
            body(ui);
        });
}

fn p5_knob<P: Param>(
    ui: &mut egui::Ui,
    label: &str,
    param: &P,
    setter: &ParamSetter,
    accent: Color32,
    width: f32,
) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, 72.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            let knob_size = 40.0;
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::splat(knob_size), Sense::click_and_drag());

            if resp.drag_started() {
                setter.begin_set_parameter(param);
            }
            if resp.dragged() {
                let delta = -resp.drag_delta().y * 0.005;
                let cur = param.unmodulated_normalized_value();
                let next = (cur + delta).clamp(0.0, 1.0);
                let plain = param.preview_plain(next);
                setter.set_parameter(param, plain);
            }
            if resp.drag_stopped() {
                setter.end_set_parameter(param);
            }
            if resp.double_clicked() {
                setter.begin_set_parameter(param);
                setter.set_parameter(param, param.default_plain_value());
                setter.end_set_parameter(param);
            }

            let norm = param.modulated_normalized_value().clamp(0.0, 1.0);
            let p = ui.painter();
            let center = rect.center();
            let r = knob_size * 0.44;

            p.circle_filled(center + Vec2::new(2.0, 2.0), r, P5_BLACK);
            p.circle_filled(center, r, Color32::from_rgb(24, 24, 30));
            p.circle_stroke(center, r, Stroke::new(1.5_f32, P5_WHITE));

            let start_ang = 0.75 * PI;
            let sweep = 1.5 * PI;
            let end_ang = start_ang + norm * sweep;

            let bg_pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let a = start_ang + (i as f32 / 24.0) * sweep;
                    Pos2::new(center.x + a.cos() * (r + 3.0), center.y + a.sin() * (r + 3.0))
                })
                .collect();
            p.add(Shape::line(
                bg_pts,
                Stroke::new(3.0_f32, Color32::from_rgb(48, 48, 58)),
            ));

            let active_steps = ((norm * 24.0).round() as usize).max(1);
            let fg_pts: Vec<Pos2> = (0..=active_steps)
                .map(|i| {
                    let a = start_ang + (i as f32 / active_steps as f32) * (norm * sweep);
                    Pos2::new(center.x + a.cos() * (r + 3.0), center.y + a.sin() * (r + 3.0))
                })
                .collect();
            p.add(Shape::line(fg_pts, Stroke::new(3.2_f32, accent)));

            let ptr_end = Pos2::new(
                center.x + end_ang.cos() * (r - 3.0),
                center.y + end_ang.sin() * (r - 3.0),
            );
            p.line_segment([center, ptr_end], Stroke::new(2.2_f32, P5_WHITE));

            let (lbl_r, _) = ui.allocate_exact_size(Vec2::new(width, 18.0), Sense::hover());
            draw_p5_3d_text(
                ui.painter(),
                lbl_r.center(),
                egui::Align2::CENTER_CENTER,
                label,
                12.0,
                P5_WHITE,
                Color32::from_rgb(180, 180, 195),
                FONT_IMPACT,
            );
            ui.label(
                RichText::new(param.to_string())
                    .size(10.0)
                    .strong()
                    .color(accent),
            );
        },
    );
}

fn draw_star_outline(
    painter: &egui::Painter,
    center: Pos2,
    outer_r: f32,
    inner_r: f32,
    rotation: f32,
    stroke: Stroke,
) {
    let mut pts = Vec::with_capacity(10);
    for i in 0..10 {
        let ang = rotation + (i as f32) * (TAU / 10.0);
        let r = if i % 2 == 0 { outer_r } else { inner_r };
        pts.push(Pos2::new(center.x + ang.cos() * r, center.y + ang.sin() * r));
    }
    painter.add(Shape::closed_line(pts, stroke));
}

fn bool_toggle(ui: &mut egui::Ui, label: &str, param: &BoolParam, setter: &ParamSetter) {
    let active = param.value();
    let btn = egui::Button::new(
        RichText::new(label)
            .size(11.0)
            .strong()
            .color(if active { P5_WHITE } else { P5_MUTED }),
    )
    .fill(if active { P5_RED } else { Color32::from_rgb(28, 28, 35) })
    .stroke(Stroke::new(
        1.5_f32,
        if active { P5_WHITE } else { Color32::from_rgb(76, 76, 90) },
    ));

    if ui.add(btn).clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, !active);
        setter.end_set_parameter(param);
    }
}

fn enum_combo_compact<T: Enum + PartialEq + Copy + 'static>(
    ui: &mut egui::Ui,
    id: &str,
    param: &EnumParam<T>,
    setter: &ParamSetter,
    variants: &[T],
    width: f32,
) {
    let current = param.value();
    let current_name = T::variants()[current.to_index()];

    egui::ComboBox::from_id_salt(id)
        .width(width)
        .selected_text(RichText::new(current_name).size(11.0).color(P5_WHITE))
        .show_ui(ui, |ui| {
            for &v in variants {
                let name = T::variants()[v.to_index()];
                if ui.selectable_label(current == v, name).clicked() {
                    setter.begin_set_parameter(param);
                    setter.set_parameter(param, v);
                    setter.end_set_parameter(param);
                }
            }
        });
}

fn is_black_key(midi: u8) -> bool {
    matches!(midi % 12, 1 | 3 | 6 | 8 | 10)
}

fn draw_piano_monitor(
    ui: &mut egui::Ui,
    in_mask: u128,
    out_mask: u128,
    ui_state: &SharedUiState,
    state: &mut EditorUiState,
) {
    let start_midi: u8 = 36;
    let end_midi: u8 = 84;
    let white_count = (start_midi..=end_midi).filter(|&m| !is_black_key(m)).count();

    egui::Frame::new()
        .fill(P5_PANEL)
        .stroke(Stroke::new(2.0_f32, P5_WHITE))
        .inner_margin(6.0)
        .show(ui, |ui| {
            let (bar_r, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
            draw_p5_3d_text(
                ui.painter(),
                Pos2::new(bar_r.left() + 4.0, bar_r.center().y),
                egui::Align2::LEFT_CENTER,
                "Live Monitor",
                13.5,
                P5_WHITE,
                P5_WHITE,
                FONT_IMPACT,
            );
            ui.painter().text(
                Pos2::new(bar_r.left() + 114.0, bar_r.center().y),
                egui::Align2::LEFT_CENTER,
                "CLICK KEYS TO PLAY",
                FontId::proportional(10.0),
                P5_MUTED,
            );
            ui.painter().text(
                Pos2::new(bar_r.right() - 4.0, bar_r.center().y),
                egui::Align2::RIGHT_CENTER,
                "■ SOUND   ■ BASS   ● INPUT",
                FontId::proportional(10.0),
                P5_GOLD,
            );

            let avail = ui.available_width();
            let height = 68.0;
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(avail, height), Sense::click_and_drag());
            let painter = ui.painter_at(rect);

            let white_w = rect.width() / white_count as f32;
            let black_w = white_w * 0.62;
            let black_h = rect.height() * 0.60;

            let mut white_rects: Vec<(u8, Rect)> = Vec::with_capacity(32);
            let mut black_rects: Vec<(u8, Rect)> = Vec::with_capacity(24);

            let mut w_idx = 0usize;
            for midi in start_midi..=end_midi {
                if !is_black_key(midi) {
                    let x0 = rect.left() + w_idx as f32 * white_w;
                    let kr = Rect::from_min_size(
                        Pos2::new(x0, rect.top()),
                        Vec2::new(white_w - 1.0, rect.height()),
                    );
                    white_rects.push((midi, kr));
                    w_idx += 1;
                } else {
                    let x_center = rect.left() + w_idx as f32 * white_w;
                    let kr = Rect::from_min_size(
                        Pos2::new(x_center - black_w * 0.5, rect.top()),
                        Vec2::new(black_w, black_h),
                    );
                    black_rects.push((midi, kr));
                }
            }

            let pointer_down = ui.input(|i| i.pointer.primary_down());
            if (response.hovered() || response.is_pointer_button_down_on()) && pointer_down {
                if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                    let hit = black_rects
                        .iter()
                        .find(|(_, r)| r.contains(pos))
                        .or_else(|| white_rects.iter().find(|(_, r)| r.contains(pos)))
                        .map(|(m, _)| *m);

                    if let Some(note) = hit {
                        if state.auditioning_note != Some(note) {
                            if let Some(prev) = state.auditioning_note {
                                ui_state.ui_audition_off.store(prev as i8, Ordering::Relaxed);
                            }
                            ui_state.ui_audition_on.store(note as i8, Ordering::Relaxed);
                            state.auditioning_note = Some(note);
                        }
                    }
                }
            } else if let Some(prev) = state.auditioning_note.take() {
                ui_state.ui_audition_off.store(prev as i8, Ordering::Relaxed);
            }

            for &(midi, kr) in &white_rects {
                let is_out = (out_mask & (1u128 << midi)) != 0;
                let is_in = (in_mask & (1u128 << midi)) != 0;
                let fill = if is_out {
                    if midi < 48 { P5_GOLD } else { P5_RED }
                } else {
                    P5_WHITE
                };
                painter.rect_filled(kr, CornerRadius::same(2), fill);
                painter.rect_stroke(
                    kr,
                    CornerRadius::same(2),
                    Stroke::new(1.0_f32, P5_BLACK),
                    StrokeKind::Inside,
                );
                if is_in {
                    painter.circle_filled(
                        Pos2::new(kr.center().x, kr.bottom() - 8.0),
                        4.0,
                        P5_CYAN,
                    );
                }
            }

            for &(midi, kr) in &black_rects {
                let is_out = (out_mask & (1u128 << midi)) != 0;
                let is_in = (in_mask & (1u128 << midi)) != 0;
                let fill = if is_out {
                    if midi < 48 { P5_GOLD } else { P5_RED }
                } else {
                    Color32::from_rgb(20, 20, 26)
                };
                painter.rect_filled(kr, CornerRadius::same(2), fill);
                painter.rect_stroke(
                    kr,
                    CornerRadius::same(2),
                    Stroke::new(1.0_f32, Color32::from_rgb(72, 72, 86)),
                    StrokeKind::Inside,
                );
                if is_in {
                    painter.circle_filled(
                        Pos2::new(kr.center().x, kr.bottom() - 7.0),
                        3.5,
                        P5_CYAN,
                    );
                }
            }
        });
}
