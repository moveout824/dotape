use eframe::egui::*;

pub const BG: Color32 = Color32::from_rgb(23, 28, 31);
pub const SURFACE: Color32 = Color32::from_rgb(29, 35, 37);
pub const RAISED: Color32 = Color32::from_rgb(39, 47, 48);
pub const TEXT: Color32 = Color32::from_rgb(224, 218, 197);
pub const MUTED: Color32 = Color32::from_rgb(146, 156, 151);
pub const ACCENT: Color32 = Color32::from_rgb(222, 173, 117);
pub const BORDER: Color32 = Color32::from_rgb(57, 67, 65);
pub const RED: Color32 = Color32::from_rgb(224, 126, 119);
pub const TRACK_COLORS: [Color32; 5] = [
    ACCENT,
    Color32::from_rgb(145, 186, 162),
    Color32::from_rgb(149, 180, 203),
    Color32::from_rgb(198, 152, 159),
    Color32::from_rgb(177, 161, 201),
];
pub fn track_color(index: usize) -> Color32 {
    TRACK_COLORS[index % TRACK_COLORS.len()]
}

pub fn setup(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "galmuri".into(),
        FontData::from_static(include_bytes!("../assets/fonts/Galmuri11.ttf")).into(),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "galmuri".into());
    }
    ctx.set_fonts(fonts);
    let mut style = (*ctx.style()).clone();
    style.visuals = Visuals::dark();
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = SURFACE;
    style.visuals.extreme_bg_color = BG;
    style.visuals.faint_bg_color = SURFACE;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.selection.bg_fill = Color32::from_rgb(69, 65, 52);
    style.visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.inactive.bg_fill = RAISED;
    style.visuals.widgets.inactive.weak_bg_fill = SURFACE;
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 56, 54);
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(43, 50, 49);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(60, 62, 52);
    style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(54, 57, 49);
    for w in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        w.corner_radius = CornerRadius::ZERO;
        w.bg_stroke = Stroke::new(1.0_f32, BORDER);
        w.fg_stroke = Stroke::new(1.0_f32, MUTED);
    }
    style.visuals.window_corner_radius = CornerRadius::ZERO;
    style.visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.window_shadow = Shadow {
        offset: [3, 3],
        blur: 0,
        spread: 0,
        color: Color32::from_black_alpha(110),
    };
    style.spacing.item_spacing = vec2(8.0, 8.0);
    style.spacing.button_padding = vec2(10.0, 6.0);
    style.spacing.interact_size.y = 28.0;
    for text in [TextStyle::Body, TextStyle::Button, TextStyle::Small] {
        style.text_styles.insert(text, FontId::proportional(12.0));
    }
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(12.0));
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(12.0));
    style.interaction.tooltip_delay = 1.5;
    style.interaction.tooltip_grace_time = 0.0;
    style.interaction.show_tooltips_only_when_still = true;
    ctx.set_style(style);
    ctx.tessellation_options_mut(|o| o.feathering = false);
}

pub fn pixel_panel(p: &Painter, r: Rect, fill: Color32, border: Color32) {
    pixel_clip(p, r, false, false, fill, Stroke::new(1.0_f32, border));
}
pub fn pixel_clip(
    p: &Painter,
    r: Rect,
    left_cut: bool,
    right_cut: bool,
    fill: Color32,
    stroke: Stroke,
) {
    let r = Rect::from_min_max(r.min.round(), r.max.round());
    if r.width() < 12.0 || r.height() < 12.0 {
        p.rect_filled(r, 0.0, fill);
        p.rect_stroke(r, 0.0, stroke, StrokeKind::Inside);
        return;
    }
    let l = if left_cut { 0.0 } else { 4.0 };
    let rr = if right_cut { 0.0 } else { 4.0 };
    p.rect_filled(
        Rect::from_min_max(r.min + vec2(l, 0.0), r.max - vec2(rr, 0.0)),
        0.0,
        fill,
    );
    p.rect_filled(
        Rect::from_min_max(r.min + vec2(l * 0.5, 2.0), r.max - vec2(rr * 0.5, 2.0)),
        0.0,
        fill,
    );
    p.rect_filled(
        Rect::from_min_max(r.min + vec2(0.0, 4.0), r.max - vec2(0.0, 4.0)),
        0.0,
        fill,
    );
    let mut points = vec![pos2(r.left() + l, r.top()), pos2(r.right() - rr, r.top())];
    if !right_cut {
        points.extend([
            pos2(r.right() - 4.0, r.top() + 2.0),
            pos2(r.right() - 2.0, r.top() + 2.0),
            pos2(r.right() - 2.0, r.top() + 4.0),
            pos2(r.right(), r.top() + 4.0),
        ]);
    }
    points.push(pos2(r.right(), r.bottom() - rr));
    if !right_cut {
        points.extend([
            pos2(r.right() - 2.0, r.bottom() - 4.0),
            pos2(r.right() - 2.0, r.bottom() - 2.0),
            pos2(r.right() - 4.0, r.bottom() - 2.0),
            pos2(r.right() - 4.0, r.bottom()),
        ]);
    }
    points.push(pos2(r.left() + l, r.bottom()));
    if !left_cut {
        points.extend([
            pos2(r.left() + 4.0, r.bottom() - 2.0),
            pos2(r.left() + 2.0, r.bottom() - 2.0),
            pos2(r.left() + 2.0, r.bottom() - 4.0),
            pos2(r.left(), r.bottom() - 4.0),
        ]);
    }
    points.push(pos2(r.left(), r.top() + l));
    if !left_cut {
        points.extend([
            pos2(r.left() + 2.0, r.top() + 4.0),
            pos2(r.left() + 2.0, r.top() + 2.0),
            pos2(r.left() + 4.0, r.top() + 2.0),
            pos2(r.left() + 4.0, r.top()),
        ]);
    }
    p.add(Shape::closed_line(points, stroke));
}

#[derive(Clone, Copy)]
pub enum Icon {
    Play,
    Pause,
    Undo,
    Redo,
    Cut,
    Settings,
    Volume,
    Muted,
    Magnet,
    Record,
    Stop,
}
pub fn icon(p: &Painter, center: Pos2, size: f32, kind: Icon, color: Color32) {
    let rows: &[&str] = match kind {
        Icon::Play => &[
            "...##.......",
            "...###......",
            "...####.....",
            "...#####....",
            "...######...",
            "...#######..",
            "...#######..",
            "...######...",
            "...#####....",
            "...####.....",
            "...###......",
            "...##.......",
        ],
        Icon::Pause => &[
            "..###..###..",
            "..###..###..",
            "..###..###..",
            "..###..###..",
            "..###..###..",
            "..###..###..",
            "..###..###..",
            "..###..###..",
            "..###..###..",
            "..###..###..",
            "..###..###..",
            "..###..###..",
        ],
        Icon::Undo | Icon::Redo => &[
            "....##......",
            "...##.......",
            "..########..",
            ".##########.",
            "..##.....###",
            "...##.....##",
            "....##....##",
            "..........##",
            ".........###",
            "....#######.",
            "....######..",
            "............",
        ],
        Icon::Cut => &[
            ".###....##..",
            "##.##..##...",
            "##.##.##....",
            ".######.....",
            "...###......",
            "....##......",
            "...###......",
            ".######.....",
            "##.##.##....",
            "##.##..##...",
            ".###....##..",
            "............",
        ],
        Icon::Settings => &[
            "....####....",
            ".##.####.##.",
            ".##########.",
            "..###..###..",
            "####....####",
            "####....####",
            "####....####",
            "####....####",
            "..###..###..",
            ".##########.",
            ".##.####.##.",
            "....####....",
        ],
        Icon::Volume | Icon::Muted => &[
            ".....##.....",
            "....###..#..",
            "...####...#.",
            ".######....#",
            ".######..#.#",
            ".######..#.#",
            ".######..#.#",
            ".######..#.#",
            ".######....#",
            "...####...#.",
            "....###..#..",
            ".....##.....",
        ],
        Icon::Magnet => &[
            ".###....###.",
            ".###....###.",
            "............",
            ".###....###.",
            ".###....###.",
            ".###....###.",
            ".###....###.",
            ".###....###.",
            ".####..####.",
            "..########..",
            "...######...",
            "....####....",
        ],
        Icon::Record => &[
            "...######...",
            "..########..",
            ".##########.",
            "############",
            "############",
            "############",
            "############",
            "############",
            "############",
            ".##########.",
            "..########..",
            "...######...",
        ],
        Icon::Stop => &[
            "............",
            ".##########.",
            ".##########.",
            ".##########.",
            ".##########.",
            ".##########.",
            ".##########.",
            ".##########.",
            ".##########.",
            ".##########.",
            ".##########.",
            "............",
        ],
    };
    let unit = (size / 12.0).round().max(1.0);
    let origin = (center - vec2(unit * 6.0, unit * 6.0)).round();
    for (y, row) in rows.iter().enumerate() {
        for (x, b) in row.bytes().enumerate() {
            if b == b'#' {
                let x = if matches!(kind, Icon::Redo) {
                    11 - x
                } else {
                    x
                };
                p.rect_filled(
                    Rect::from_min_size(
                        origin + vec2(x as f32 * unit, y as f32 * unit),
                        vec2(unit, unit),
                    ),
                    0.0,
                    color,
                );
            }
        }
    }
    if matches!(kind, Icon::Muted) {
        for i in 0..10 {
            let at = origin + vec2((i + 1) as f32 * unit, (i + 1) as f32 * unit);
            p.rect_filled(
                Rect::from_min_size(at, vec2(unit * 1.5, unit * 1.5)),
                0.0,
                RED,
            );
        }
    }
}
pub fn icon_button(ui: &mut Ui, kind: Icon, hint: &str) -> Response {
    let (r, response) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
    if response.hovered() {
        pixel_panel(ui.painter(), r, RAISED, BORDER);
    }
    icon(
        ui.painter(),
        r.center(),
        19.0,
        kind,
        if ui.is_enabled() {
            TEXT
        } else {
            MUTED.gamma_multiply(0.5)
        },
    );
    response.on_hover_text(hint)
}
pub fn mini_icon_button(ui: &mut Ui, kind: Icon, hint: &str) -> Response {
    let (r, response) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(r, 0.0, RAISED);
    }
    icon(
        ui.painter(),
        r.center(),
        17.0,
        kind,
        if matches!(kind, Icon::Muted) {
            RED
        } else {
            MUTED
        },
    );
    response.on_hover_text(hint)
}

pub fn double_click(response: &Response) -> bool {
    if !response.clicked_by(PointerButton::Primary) {
        return false;
    }
    let now = response.ctx.input(|i| i.time);
    let pointer = response
        .interact_pointer_pos()
        .unwrap_or(response.rect.center());
    let key = response.id.with("local-double-click");
    response.ctx.data_mut(|data| {
        let previous = data.get_temp::<(f64, Pos2)>(key);
        if previous.is_some_and(|(time, pos)| now - time < 0.32 && pointer.distance(pos) < 8.0) {
            data.remove::<(f64, Pos2)>(key);
            true
        } else {
            data.insert_temp(key, (now, pointer));
            false
        }
    })
}

pub fn primary(ui: &mut Ui, label: &str, size: Vec2) -> Response {
    ui.add_sized(
        size,
        Button::new(RichText::new(label).color(BG).strong())
            .fill(ACCENT)
            .stroke(Stroke::NONE)
            .corner_radius(0),
    )
}

/// Explicit contrasting rail and handle; numbers and rail both reset on double click.
pub fn value_slider(
    ui: &mut Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    default: f32,
    width: f32,
    percent: bool,
) -> Response {
    let id = ui.next_auto_id();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 28.0), Sense::hover());
    let number_width = if percent { 43.0 } else { 70.0 };
    let number = Rect::from_min_max(pos2(rect.right() - number_width, rect.top()), rect.max);
    let rail = Rect::from_min_max(
        rect.min + vec2(5.0, 0.0),
        pos2(number.left() - 10.0, rect.bottom()),
    );
    let mut response = ui.interact(
        rail.expand2(vec2(4.0, 0.0)),
        id.with("rail"),
        Sense::click_and_drag(),
    );
    let before = *value;
    if response.is_pointer_button_down_on() {
        if let Some(pointer) = response.interact_pointer_pos() {
            *value = *range.start()
                + ((pointer.x - rail.left()) / rail.width()).clamp(0.0, 1.0)
                    * (*range.end() - *range.start());
        }
    }
    let numeric = ui.interact(number, id.with("number"), Sense::click_and_drag());
    if numeric.dragged_by(PointerButton::Primary) {
        *value = (*value + numeric.drag_delta().x * if percent { 0.01 } else { 0.1 })
            .clamp(*range.start(), *range.end());
    }
    if numeric.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    let reset = double_click(&response) || double_click(&numeric);
    if reset {
        *value = default;
        response.surrender_focus();
        numeric.surrender_focus();
    }
    response = response.union(numeric);
    if (*value - before).abs() > f32::EPSILON || reset {
        response.mark_changed();
    }
    let fraction = ((*value - *range.start()) / (*range.end() - *range.start())).clamp(0.0, 1.0);
    let y = rail.center().y;
    let knob = pos2(rail.left() + fraction * rail.width(), y);
    ui.painter().line_segment(
        [pos2(rail.left(), y), pos2(rail.right(), y)],
        Stroke::new(3.0_f32, Color32::from_rgb(80, 86, 102)),
    );
    ui.painter()
        .line_segment([pos2(rail.left(), y), knob], Stroke::new(3.0_f32, ACCENT));
    ui.painter().rect_filled(
        Rect::from_center_size(knob.round(), vec2(6.0, 10.0)),
        0.0,
        TEXT,
    );
    ui.painter()
        .rect_filled(number.shrink2(vec2(0.0, 2.0)), 0.0, RAISED);
    let text = if percent {
        format!("{:.0}%", *value * 100.0)
    } else {
        format!("{:.1} dB", *value)
    };
    ui.painter().with_clip_rect(number).text(
        number.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::monospace(12.0),
        TEXT,
    );
    response
}

pub fn reset_combo<T: Copy + PartialEq>(
    value: &mut T,
    default: T,
    mut response: Response,
) -> Response {
    if double_click(&response) {
        *value = default;
        response.mark_changed();
        Popup::close_all(&response.ctx);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_combo_double_click_restores_default_even_with_popup() {
        let ctx = Context::default();
        let mut value = 96000_u32;
        let mut time = 0.0;
        let mut frame = |events: Vec<Event>| {
            time += 0.04;
            let mut face = Rect::NOTHING;
            let _ = ctx.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 350.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    CentralPanel::default().show(ctx, |ui| {
                        let r = ComboBox::from_id_salt("rate-test")
                            .selected_text(value.to_string())
                            .show_ui(ui, |ui| {
                                for n in [44100, 48000, 96000] {
                                    ui.selectable_value(&mut value, n, n.to_string());
                                }
                            })
                            .response;
                        face = r.rect;
                        reset_combo(&mut value, 48000, r);
                    });
                },
            );
            face
        };
        let p = frame(vec![]).center();
        for _ in 0..2 {
            frame(vec![Event::PointerMoved(p)]);
            frame(vec![Event::PointerButton {
                pos: p,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }]);
            frame(vec![Event::PointerButton {
                pos: p,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }]);
        }
        drop(frame);
        assert_eq!(value, 48000);
    }
}
