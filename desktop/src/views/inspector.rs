use eframe::egui;
use egui::{Color32, RichText, Sense, Stroke, Vec2};

use crate::app::App;
use crate::app::live::selector_args;
use crate::components::*;
use crate::models::UiNode;
use crate::theme;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    page_header(
        ui,
        "Inspector",
        "A periodic screenshot with a semantic UIAutomator hierarchy. Preview is not video streaming.",
    );
    let sessions = app.sessions.value.sessions.clone();
    card(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Session").size(10.0).color(theme::MUTED));
            let current = app
                .inspector
                .session_id
                .clone()
                .unwrap_or_else(|| "Select a session".to_string());
            egui::ComboBox::from_id_salt("inspector-session")
                .width(250.0)
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for session in &sessions {
                        if ui
                            .selectable_label(
                                app.inspector.session_id.as_deref() == Some(session.id.as_str()),
                                format!(
                                    "{} · {} · {}",
                                    session.serial.as_deref().unwrap_or("-"),
                                    session.id,
                                    session.state
                                ),
                            )
                            .clicked()
                        {
                            app.inspector_select_session(Some(session.id.clone()));
                            app.start_inspector(ui.ctx());
                        }
                    }
                });
            if secondary_button(ui, "Refresh").clicked() {
                app.refresh_inspector(ui.ctx());
            }
            if secondary_button(
                ui,
                if app.inspector.auto_refresh {
                    "Pause preview"
                } else {
                    "Resume preview"
                },
            )
            .clicked()
            {
                app.inspector_pause();
            }
            ui.checkbox(&mut app.inspector.auto_refresh, "Auto refresh");
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Preview every")
                    .size(10.0)
                    .color(theme::MUTED),
            );
            ui.scope(|ui| {
                theme::configure_slider(ui, 260.0);
                ui.add_sized(
                    [320.0, 22.0],
                    egui::Slider::new(&mut app.inspector.interval_ms, 500..=5000)
                        .suffix(" ms")
                        .trailing_fill(true)
                        .show_value(true),
                );
            });
            if app.inspector.frame_loading {
                ui.spinner();
                small(ui, "capturing frame");
            }
            if app.inspector.hierarchy_loading {
                ui.spinner();
                small(ui, "dumping hierarchy");
            }
        });
        if app.inspector.frame_error.is_some() || app.inspector.hierarchy_error.is_some() {
            let frame_error = app.inspector.frame_error.as_deref();
            let hierarchy_error = app.inspector.hierarchy_error.as_deref();
            if let Some(error) = frame_error {
                error_banner(ui, error);
            }
            if let Some(error) = hierarchy_error
                && Some(error) != frame_error
            {
                error_banner(ui, error);
            }
        }
    });
    ui.add_space(theme::PAGE_GAP);
    performance(app, ui);
    ui.add_space(theme::PAGE_GAP);
    let total = ui.available_width();
    let preview_width = (total * 0.38).max(340.0);
    let hierarchy_width = (total - preview_width - theme::PAGE_GAP).max(380.0);
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(preview_width, 700.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                card_fixed_height(ui, 700.0, "inspector-preview-card", |ui| {
                    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                        preview(app, ui);
                    });
                });
            },
        );
        ui.add_space(theme::PAGE_GAP);
        ui.allocate_ui_with_layout(
            Vec2::new(hierarchy_width, 700.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                card_fixed_height(ui, 700.0, "inspector-hierarchy-card", |ui| {
                    hierarchy(app, ui);
                });
            },
        );
    });
    if let Some(node) = app.selected_node().cloned() {
        ui.add_space(theme::PAGE_GAP);
        node_details(app, ui, &node);
    }
}

fn preview(app: &mut App, ui: &mut egui::Ui) {
    section_label(ui, "Device preview");
    let Some(texture) = app.inspector.image.clone() else {
        card(ui, |ui| {
            empty_state(
                ui,
                "No preview frame",
                "Select a session and refresh. The device must be awake and authorized.",
            );
            if app.inspector.frame_loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Capturing frame...");
                });
            }
        });
        return;
    };
    let (image_w, image_h) = app.inspector.image_size.unwrap_or((1080, 2400));
    let available = ui.available_size();
    let max_h = 600.0;
    let scale = (available.x / image_w as f32)
        .min(max_h / image_h as f32)
        .max(0.05);
    let size = Vec2::new(image_w as f32 * scale, image_h as f32 * scale);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let (display_w, display_h) = app.inspector.display_size.unwrap_or((image_w, image_h));
    let rotation = app
        .inspector
        .hierarchy
        .as_ref()
        .and_then(|hierarchy| hierarchy.rotation)
        .unwrap_or(0);
    let rotation_odd = matches!(rotation, 1 | 3);
    let (base_w, base_h) = if rotation_odd {
        (display_h, display_w)
    } else {
        (display_w, display_h)
    };
    let scale_x = rect.width() / base_w.max(1) as f32;
    let scale_y = rect.height() / base_h.max(1) as f32;
    ui.painter()
        .rect_filled(rect, 4.0, Color32::from_rgb(5, 8, 12));
    ui.painter().image(
        texture.id(),
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    if let Some(error) = app.inspector.frame_error.as_deref()
        && (error.contains("not connected") || error.contains("E_DEVICE_NOT_FOUND"))
    {
        ui.painter()
            .rect_filled(rect, 4.0, Color32::from_rgba_unmultiplied(5, 8, 12, 190));
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Device disconnected",
            egui::FontId::proportional(15.0),
            theme::RED,
        );
        ui.painter().text(
            rect.center() + Vec2::new(0.0, 20.0),
            egui::Align2::CENTER_CENTER,
            "Showing the last captured frame",
            egui::FontId::proportional(10.0),
            theme::MUTED,
        );
    }
    let selected = app.selected_node().cloned();
    if let Some(node) = &selected {
        let highlight = egui::Rect::from_min_max(
            egui::pos2(
                rect.left() + node.bounds.left as f32 * scale_x,
                rect.top() + node.bounds.top as f32 * scale_y,
            ),
            egui::pos2(
                rect.left() + node.bounds.right as f32 * scale_x,
                rect.top() + node.bounds.bottom as f32 * scale_y,
            ),
        )
        .intersect(rect);
        if !highlight.is_negative() {
            ui.painter().rect_stroke(
                highlight,
                2.0,
                Stroke::new(2.0, theme::accent()),
                egui::StrokeKind::Inside,
            );
        }
    }
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(hierarchy) = &app.inspector.hierarchy
    {
        let x = ((pos.x - rect.left()) / scale_x) as i32;
        let y = ((pos.y - rect.top()) / scale_y) as i32;
        let hit = hierarchy
            .nodes
            .iter()
            .filter(|node| {
                node.bounds.left <= x
                    && node.bounds.right >= x
                    && node.bounds.top <= y
                    && node.bounds.bottom >= y
            })
            .max_by_key(|node| node.depth);
        if let Some(hit) = hit {
            app.inspector.selected_node = Some(hit.id);
        }
    }
    ui.horizontal(|ui| {
        small(
            ui,
            format!(
                "{image_w} × {image_h}{}{}",
                if rotation_odd { " · rotated" } else { "" },
                app.inspector
                    .last_frame
                    .map(|last| format!(" · updated {}s ago", last.elapsed().as_secs()))
                    .unwrap_or_default()
            ),
        );
        if app.inspector.frame_loading {
            ui.spinner();
        }
        if let Some(node) = &selected {
            small(ui, format!("selected {}", node.id));
        }
    });
}

fn hierarchy(app: &mut App, ui: &mut egui::Ui) {
    section_label(ui, "Semantic hierarchy");
    let Some(hierarchy) = app.inspector.hierarchy.clone() else {
        card(ui, |ui| {
            empty_state(
                ui,
                "No hierarchy",
                "Refresh to dump the accessibility tree from the selected device.",
            );
            if app.inspector.hierarchy_loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Dumping hierarchy...");
                });
            }
        });
        return;
    };
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut app.inspector.search)
                .desired_width(200.0)
                .hint_text("filter text, id or class"),
        );
        ui.checkbox(&mut app.inspector.raw_xml, "Raw XML");
        if app.inspector.hierarchy_loading {
            ui.spinner();
        }
        small(ui, format!("{} nodes", hierarchy.nodes.len()));
    });
    if app.inspector.raw_xml {
        egui::ScrollArea::vertical()
            .id_salt("inspector-raw")
            .max_height(420.0)
            .show(ui, |ui| {
                code_block(ui, &hierarchy.xml);
            });
        return;
    }
    let query = app.inspector.search.to_ascii_lowercase();
    let nodes = hierarchy.nodes.clone();
    let selected = app.inspector.selected_node;
    let collapsed = app.inspector.collapsed.clone();
    let roots = if let Some(root) = hierarchy.root {
        vec![root]
    } else {
        nodes
            .iter()
            .filter(|node| node.parent.is_none())
            .map(|node| node.id)
            .collect()
    };
    let mut select = None;
    let mut toggle = None;
    let mut tree = NodeTree {
        nodes: &nodes,
        collapsed: &collapsed,
        selected,
        query: &query,
        select: &mut select,
        toggle: &mut toggle,
    };
    egui::ScrollArea::vertical()
        .id_salt("inspector-nodes")
        .max_height(540.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (index, root) in roots.iter().enumerate() {
                if index > 0 {
                    ui.add_space(6.0);
                }
                node_card(ui, &mut tree, *root);
            }
        });
    if let Some(id) = toggle
        && !app.inspector.collapsed.remove(&id)
    {
        app.inspector.collapsed.insert(id);
    }
    if let Some(id) = select {
        app.inspector.selected_node = Some(id);
    }
}

struct NodeTree<'a> {
    nodes: &'a [UiNode],
    collapsed: &'a std::collections::HashSet<usize>,
    selected: Option<usize>,
    query: &'a str,
    select: &'a mut Option<usize>,
    toggle: &'a mut Option<usize>,
}

fn node_card(ui: &mut egui::Ui, tree: &mut NodeTree<'_>, id: usize) {
    let Some(node) = tree.nodes.iter().find(|node| node.id == id) else {
        return;
    };
    let node_matches = tree.query.is_empty() || node_matches_query(node, tree.query);
    let descendant_matches = !tree.query.is_empty()
        && node
            .children
            .iter()
            .any(|child| subtree_matches(tree.nodes, *child, tree.query));
    if !node_matches && !descendant_matches {
        return;
    }

    let has_children = !node.children.is_empty();
    let open = if tree.query.is_empty() {
        !tree.collapsed.contains(&id)
    } else {
        true
    };
    let state_id = ui.make_persistent_id(("inspector-node-card", id));
    let mut state =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), state_id, open);
    state.set_open(open);
    let openness = state.openness(ui.ctx());
    let selected_here = tree.selected == Some(id);
    let dark = ui.style().visuals.dark_mode;
    let fill = if selected_here {
        theme::accent_soft()
    } else {
        theme::row_fill(ui)
    };
    let border = if selected_here {
        theme::accent()
    } else {
        theme::border_color(ui)
    };
    let response = egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(theme::CARD_RADIUS)
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            let header = node_header(ui, node, has_children, openness, selected_here);
            if has_children {
                state.show_body_unindented(ui, |ui| {
                    ui.add_space(4.0);
                    egui::Frame::new()
                        .fill(if dark {
                            theme::DARK_PANEL
                        } else {
                            Color32::from_rgb(235, 240, 247)
                        })
                        .stroke(Stroke::new(1.0, theme::border_color(ui)))
                        .corner_radius(theme::ROW_RADIUS)
                        .inner_margin(egui::Margin::symmetric(8, 8))
                        .show(ui, |ui| {
                            for (index, child) in node.children.iter().enumerate() {
                                if index > 0 {
                                    ui.add_space(4.0);
                                }
                                node_card(ui, tree, *child);
                            }
                        });
                });
            }
            header
        })
        .inner;
    if response.clicked() {
        *tree.select = Some(id);
        if has_children {
            *tree.toggle = Some(id);
        }
    }
}

fn node_header(
    ui: &mut egui::Ui,
    node: &UiNode,
    has_children: bool,
    openness: f32,
    selected: bool,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
    if response.hovered() && !selected {
        ui.painter()
            .rect_filled(rect, theme::ROW_RADIUS, theme::row_hover_fill(ui));
    }
    if has_children {
        paint_chevron(
            ui.painter(),
            rect.left() + 14.0,
            rect.center().y,
            openness,
            if selected {
                theme::accent()
            } else {
                theme::MUTED
            },
        );
    }
    let mut content = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(10.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    content.add_space(18.0);
    content.add(
        egui::Label::new(
            RichText::new(node_label(node))
                .size(10.5)
                .color(theme::text_color(ui)),
        )
        .truncate(),
    );
    response.on_hover_text(node_label(node))
}

fn paint_chevron(painter: &egui::Painter, x: f32, center_y: f32, openness: f32, color: Color32) {
    let t = openness.clamp(0.0, 1.0);
    let p1 = egui::pos2(x - 3.0, center_y - 4.0 + 1.5 * t);
    let p2 = egui::pos2(x + 1.0 + 3.0 * t, center_y - 0.5 + 0.5 * t);
    let p3 = egui::pos2(x - 3.0, center_y + 4.0 - 1.5 * t);
    let stroke = Stroke::new(1.6, color);
    painter.line_segment([p1, p2], stroke);
    painter.line_segment([p2, p3], stroke);
}

fn node_matches_query(node: &UiNode, query: &str) -> bool {
    format!(
        "{} {} {} {}",
        node.text.as_deref().unwrap_or(""),
        node.content_desc.as_deref().unwrap_or(""),
        node.resource_id.as_deref().unwrap_or(""),
        node.class.as_deref().unwrap_or("")
    )
    .to_ascii_lowercase()
    .contains(query)
}

fn subtree_matches(nodes: &[UiNode], id: usize, query: &str) -> bool {
    let Some(node) = nodes.iter().find(|node| node.id == id) else {
        return false;
    };
    node_matches_query(node, query)
        || node
            .children
            .iter()
            .any(|child| subtree_matches(nodes, *child, query))
}

fn node_details(app: &mut App, ui: &mut egui::Ui, node: &UiNode) {
    card(ui, |ui| {
        section_title(ui, "Selected node");
        kv_grid(
            ui,
            "node-details",
            &[
                ("Text", node.text.clone().unwrap_or_else(|| "-".to_string())),
                (
                    "Content desc",
                    node.content_desc.clone().unwrap_or_else(|| "-".to_string()),
                ),
                (
                    "Resource id",
                    node.resource_id.clone().unwrap_or_else(|| "-".to_string()),
                ),
                (
                    "Class",
                    node.class.clone().unwrap_or_else(|| "-".to_string()),
                ),
                (
                    "Package",
                    node.package.clone().unwrap_or_else(|| "-".to_string()),
                ),
                ("Bounds", node.bounds_raw.clone()),
                (
                    "Flags",
                    format!(
                        "clickable={} long_clickable={} enabled={} focused={} selected={} scrollable={} checked={}",
                        node.clickable,
                        node.long_clickable,
                        node.enabled,
                        node.focused,
                        node.selected,
                        node.scrollable,
                        node.checked
                    ),
                ),
            ],
        );
        let selector = selector_args(node);
        let has_selector = !selector.is_empty();
        ui.horizontal_wrapped(|ui| {
            if !has_selector {
                warning_banner(ui, "No semantic selector; coordinate fallback is explicit.");
            } else {
                copy_button(ui, &selector.join(" "), "Copy selector");
            }
            if action_button(
                ui,
                "Tap",
                has_selector && !app.inspector.action_busy,
                "Tap this node using its semantic selector",
            )
            .clicked()
            {
                app.inspector_tap(ui.ctx());
            }
            if action_button(
                ui,
                "Long press",
                has_selector && !app.inspector.action_busy,
                "Long-press this node using its semantic selector",
            )
            .clicked()
            {
                app.inspector_long_press(ui.ctx());
            }
            if action_button(
                ui,
                "Coordinate tap",
                !app.inspector.action_busy && !has_selector,
                "Fallback only: tap the node bounds center",
            )
            .clicked()
            {
                app.inspector_coordinate_tap(ui.ctx());
            }
            if action_button(ui, "Back", !app.inspector.action_busy, "Press Android Back").clicked()
            {
                app.inspector_back(ui.ctx());
            }
            if action_button(ui, "Home", !app.inspector.action_busy, "Press Android Home").clicked()
            {
                app.inspector_home(ui.ctx());
            }
            if action_button(
                ui,
                "Scroll up",
                !app.inspector.action_busy,
                "Scroll the selected or default scrollable up",
            )
            .clicked()
            {
                app.inspector_scroll(ui.ctx(), "up");
            }
            if action_button(
                ui,
                "Scroll down",
                !app.inspector.action_busy,
                "Scroll the selected or default scrollable down",
            )
            .clicked()
            {
                app.inspector_scroll(ui.ctx(), "down");
            }
        });
        if node
            .class
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase()
            .contains("edittext")
        {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut app.inspector.typed_text)
                        .desired_width(220.0)
                        .hint_text("text to type"),
                );
                if primary_button(ui, "Type").clicked() {
                    app.inspector_type(ui.ctx());
                }
            });
        }
        if app.inspector.action_busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Applying action...");
            });
        }
        if let Some(action) = &app.inspector.last_action {
            small(ui, action);
        }
    });
}

fn node_label(node: &UiNode) -> String {
    let text = node
        .text
        .as_deref()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            node.content_desc
                .as_deref()
                .filter(|value| !value.is_empty())
        })
        .or_else(|| {
            node.resource_id
                .as_deref()
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| node.class.as_deref().unwrap_or("node"));
    format!(
        "{}  {}{}",
        if node.clickable { "◉" } else { "·" },
        text,
        if node.scrollable { "  ↕" } else { "" }
    )
}

fn performance(app: &mut App, ui: &mut egui::Ui) {
    section_label(ui, "Performance evidence");
    card(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            let can_measure =
                app.inspector.session_id.is_some() && !app.inspector.performance_loading;
            if action_button(
                ui,
                "Measure",
                can_measure,
                "Run diagnostics performance for this session",
            )
            .clicked()
            {
                app.inspector_measure_performance(ui.ctx());
            }
            ui.checkbox(
                &mut app.inspector.performance_cold,
                "Cold start (force-stop)",
            );
            if app.inspector.performance_loading {
                ui.spinner();
                small(ui, "measuring startup, jank, memory and CPU");
            }
        });
        if let Some(error) = &app.inspector.performance_error {
            error_banner(ui, error);
        }
        let Some(report) = &app.inspector.performance else {
            small(
                ui,
                "No measurement yet. A cold measurement force-stops the app but preserves its data.",
            );
            return;
        };
        ui.horizontal_wrapped(|ui| {
            let startup = match report.startup.total_time_ms {
                Some(total) => format!(
                    "startup {} ms{}",
                    total,
                    if report.startup.cold {
                        " cold"
                    } else {
                        " warm"
                    }
                ),
                None => format!(
                    "startup unknown: {}",
                    report
                        .startup
                        .unknown_reason
                        .as_deref()
                        .unwrap_or("no timing")
                ),
            };
            let jank = match (report.jank.total_frames, report.jank.janky_frames) {
                (Some(total), Some(janky)) => format!("jank {janky}/{total} frames"),
                _ => format!(
                    "jank unknown: {}",
                    report.jank.unknown_reason.as_deref().unwrap_or("no frames")
                ),
            };
            let memory = match report.memory.total_pss_kb {
                Some(pss) => format!("PSS {pss} KiB"),
                None => format!(
                    "memory unknown: {}",
                    report
                        .memory
                        .unknown_reason
                        .as_deref()
                        .unwrap_or("no memory data")
                ),
            };
            let cpu = match report.cpu.total_percent {
                Some(total) => format!("CPU {total:.1}%"),
                None => format!(
                    "CPU unknown: {}",
                    report
                        .cpu
                        .unknown_reason
                        .as_deref()
                        .unwrap_or("no CPU data")
                ),
            };
            for metric in [startup, jank, memory, cpu] {
                egui::Frame::new()
                    .fill(theme::surface_fill(ui))
                    .stroke(Stroke::new(1.0, theme::border_color(ui)))
                    .corner_radius(4.0)
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.label(RichText::new(metric).size(10.0));
                    });
            }
        });
        if report.cold_start_requested {
            small(
                ui,
                "Cold measurement requested: the app was force-stopped before launch.",
            );
        }
        for warning in &report.warnings {
            ui.label(RichText::new(warning).size(10.0).color(theme::YELLOW));
        }
    });
}
