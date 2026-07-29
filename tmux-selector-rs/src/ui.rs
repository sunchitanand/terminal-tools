//! ratatui rendering. Keeps the tabular layout of the zsh version: a table
//! with SESSION / STARTED / ACTIVE columns, project group headers, a running
//! dot, a bulk-select tick, plus a search line and an action bar.

use crate::app::{Action, App, Prompt, Row};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row as TRow, Table, TableState};

// Palette — warm, colorblind-safe (meaning carried by text + symbols, not hue).
const ACCENT: Color = Color::Magenta; // title / headers
const CYAN: Color = Color::Cyan; // host / running name / search
const GREEN: Color = Color::Green; // running dot / new
const DIM: Color = Color::DarkGray; // borders / inactive
const YELLOW: Color = Color::Yellow; // timestamps / picked tick

pub struct RenderCtx<'a> {
    pub app: &'a App,
    pub host_short: &'a str,
}

/// Hit-testing metadata recorded during a render pass, so the event loop can
/// map mouse coordinates back to rows and action-bar segments.
#[derive(Default)]
pub struct MouseMap {
    /// Terminal region where data rows are drawn (inside the border, below the
    /// column header).
    data_rect: Rect,
    /// Index into the rendered table rows of the first visible data row.
    offset: usize,
    /// For each rendered table row, the `App::selectable` index it maps to
    /// (spacers and group headers map to `None`).
    trow_to_selectable: Vec<Option<usize>>,
    /// Clickable action-bar segments: (start_col, end_col_exclusive, action).
    action_regions: Vec<(u16, u16, Action)>,
    /// Terminal row of the action bar.
    action_bar_y: u16,
    /// Exclusive end column of the marker (tick/dot) cell; clicks left of this
    /// within a data row toggle the bulk-pick instead of selecting.
    marker_x_end: u16,
}

impl MouseMap {
    /// Resolve a click at (col, row) to a selectable index, if it landed on a
    /// data row.
    pub fn hit_row(&self, col: u16, row: u16) -> Option<usize> {
        let r = self.data_rect;
        if col < r.x || col >= r.x + r.width {
            return None;
        }
        if row < r.y || row >= r.y + r.height {
            return None;
        }
        let k = (row - r.y) as usize;
        let trow_index = self.offset + k;
        self.trow_to_selectable.get(trow_index).copied().flatten()
    }

    /// Resolve a click to an action-bar segment.
    pub fn hit_action(&self, col: u16, row: u16) -> Option<Action> {
        if row != self.action_bar_y {
            return None;
        }
        self.action_regions
            .iter()
            .find(|(s, e, _)| col >= *s && col < *e)
            .map(|(_, _, a)| *a)
    }

    /// Did the click land on the marker (tick/dot) cell of a data row? If so,
    /// returns the selectable index whose pick-state should toggle.
    pub fn hit_marker(&self, col: u16, row: u16) -> Option<usize> {
        if col >= self.marker_x_end {
            return None;
        }
        self.hit_row(col, row)
    }
}

/// A blank table row used to separate project groups.
fn spacer_row<'a>() -> TRow<'a> {
    TRow::new(vec![Cell::from(""), Cell::from(""), Cell::from(""), Cell::from("")])
}

pub fn render(f: &mut Frame, ctx: &RenderCtx, map: &mut MouseMap) {
    let area = f.area();

    // Layout: title (2), table (fill), gap (1), action bar (1), hint (1).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    render_title(f, chunks[0], ctx);
    render_table(f, chunks[1], ctx, map);
    render_action_bar(f, chunks[3], ctx, map);
    render_hint(f, chunks[4], ctx);

    // Transient status line (e.g. "Deleting 3…") replaces the hint briefly.
    if let Some(msg) = &ctx.app.status {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(msg.clone(), Style::default().fg(YELLOW)),
            ])),
            chunks[4],
        );
    }

    // Modal prompt drawn last, over everything else.
    if ctx.app.prompt.is_active() {
        render_prompt(f, area, ctx);
    }
}

/// Center a `w`×`h` rect within `area` (clamped to fit).
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 3; // upper-third looks better
    Rect { x, y, width: w, height: h }
}

/// Draw the active modal prompt as a centered popup over the list.
fn render_prompt(f: &mut Frame, area: Rect, ctx: &RenderCtx) {
    let app = ctx.app;
    let accent = Style::default().fg(ACCENT);
    let cyan = Style::default().fg(CYAN);
    let dim = Style::default().fg(DIM);

    // Build the popup's inner lines (title, optional context, input/confirm).
    let (title, body): (String, Vec<Line>) = match &app.prompt {
        Prompt::ConfirmDelete { names } => {
            let title = "Delete".to_string();
            let mut body = Vec::new();
            if names.len() == 1 {
                body.push(Line::from(vec![
                    Span::raw("Delete "),
                    Span::styled(names[0].clone(), accent),
                    Span::raw("?"),
                ]));
            } else {
                body.push(Line::from(vec![
                    Span::raw("Delete "),
                    Span::styled(format!("{} sessions", names.len()), accent),
                    Span::raw("?"),
                ]));
                // Show up to a few names for context.
                let preview: Vec<String> = names.iter().take(4).cloned().collect();
                let mut extra = preview.join(", ");
                if names.len() > 4 {
                    extra.push_str(&format!(", +{} more", names.len() - 4));
                }
                body.push(Line::from(Span::styled(extra, dim)));
            }
            body.push(Line::from(""));
            body.push(Line::from(vec![
                Span::styled("y", cyan),
                Span::styled(" delete    ", dim),
                Span::styled("n", cyan),
                Span::styled("/", dim),
                Span::styled("esc", cyan),
                Span::styled(" cancel", dim),
            ]));
            (title, body)
        }
        Prompt::Rename { old, buffer } => {
            let body = vec![
                Line::from(vec![
                    Span::styled("renaming ", dim),
                    Span::styled(old.clone(), dim),
                ]),
                Line::from(""),
                input_line(buffer, cyan),
                Line::from(""),
                enter_esc_hint(dim, cyan),
            ];
            ("Rename".to_string(), body)
        }
        Prompt::MoveTo {
            names,
            candidates,
            selected,
        } => {
            let mut body = Vec::new();
            let label = if names.len() == 1 {
                format!("move {}", names[0])
            } else {
                format!("move {} sessions", names.len())
            };
            body.push(Line::from(Span::styled(label, dim)));
            body.push(Line::from(""));
            // One line per candidate project; the selected one is highlighted
            // with a chevron + inverse style. Up/down cycles the selection.
            for (i, proj) in candidates.iter().enumerate() {
                if i == *selected {
                    body.push(Line::from(vec![
                        Span::styled("› ", cyan),
                        Span::styled(
                            proj.clone(),
                            Style::default()
                                .bg(Color::Blue)
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));
                } else {
                    body.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(proj.clone(), Style::default().fg(Color::White)),
                    ]));
                }
            }
            body.push(Line::from(""));
            body.push(Line::from(vec![
                Span::styled("↑↓", cyan),
                Span::styled(" pick    ", dim),
                Span::styled("↵", cyan),
                Span::styled(" confirm    ", dim),
                Span::styled("esc", cyan),
                Span::styled(" cancel", dim),
            ]));
            ("Move to project".to_string(), body)
        }
        Prompt::NewSession { buffer } => {
            let body = vec![
                Line::from(Span::styled("name as project/name", dim)),
                Line::from(""),
                input_line(buffer, cyan),
                Line::from(""),
                enter_esc_hint(dim, cyan),
            ];
            ("New session".to_string(), body)
        }
        Prompt::None => return,
    };

    // Size: width fits content (min 40, max area-4), height = body + borders.
    let content_w = body
        .iter()
        .map(|l| l.width())
        .max()
        .unwrap_or(0)
        .max(title.len())
        .max(38) as u16
        + 4;
    let w = content_w.min(area.width.saturating_sub(4));
    let h = body.len() as u16 + 2; // top/bottom border
    let popup = centered(area, w, h);

    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(accent)
        .title(Span::styled(format!(" {title} "), accent.add_modifier(Modifier::BOLD)));
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    f.render_widget(Paragraph::new(body), inner);
}

/// A text-input display line: "> buffer█".
fn input_line<'a>(buffer: &str, cyan: Style) -> Line<'a> {
    Line::from(vec![
        Span::styled("> ", cyan),
        Span::styled(buffer.to_string(), Style::default().fg(Color::White)),
        Span::styled("█", cyan),
    ])
}

fn enter_esc_hint<'a>(dim: Style, cyan: Style) -> Line<'a> {
    Line::from(vec![
        Span::styled("↵", cyan),
        Span::styled(" confirm    ", dim),
        Span::styled("esc", cyan),
        Span::styled(" cancel", dim),
    ])
}

fn render_title(f: &mut Frame, area: Rect, ctx: &RenderCtx) {
    let app = ctx.app;
    let mut spans = vec![
        Span::raw("  "),
        Span::styled("⚡ tmux :: ", Style::default().fg(ACCENT)),
        Span::styled(
            ctx.host_short.to_string(),
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        ),
    ];
    if app.show_archived {
        // Clear text label — colorblind-safe, meaning is in the word.
        spans.push(Span::styled(
            "   [ARCHIVED VIEW]",
            Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
        ));
    } else {
        let n = app.archived_count();
        if n > 0 {
            spans.push(Span::styled(
                format!("   ({n} archived · ^A)"),
                Style::default().fg(DIM),
            ));
        }
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_table(f: &mut Frame, area: Rect, ctx: &RenderCtx, map: &mut MouseMap) {
    let app = ctx.app;
    let cursor_ri = app.selectable.get(app.cursor).copied();

    let mut trows: Vec<TRow> = Vec::new();
    // Index of the cursor row within the filtered (visible) table rows, so we
    // can drive TableState scrolling to keep it on screen.
    let mut cursor_visible_idx: Option<usize> = None;
    // Whether any visible row has been emitted yet (to skip the leading spacer
    // before the first group).
    let mut rendered_any = false;
    // Parallel to `trows`: the selectable index each rendered row maps to, for
    // mouse hit-testing (spacers/headers map to None).
    let mut trow_to_selectable: Vec<Option<usize>> = Vec::new();
    // Reverse lookup from a row index in `app.rows` to its selectable index.
    let selectable_of = |ri: usize| app.selectable.iter().position(|&x| x == ri);

    for (ri, row) in app.rows.iter().enumerate() {
        if !app.row_visible(ri) {
            continue;
        }
        // Blank spacer line before each group header, separating groups.
        if matches!(row, Row::Header(_)) && rendered_any {
            trows.push(spacer_row());
            trow_to_selectable.push(None);
        }
        let is_cursor = Some(ri) == cursor_ri;
        if is_cursor {
            cursor_visible_idx = Some(trows.len());
        }
        rendered_any = true;
        // Record the mapping for this about-to-be-pushed row.
        trow_to_selectable.push(match row {
            Row::New | Row::Session(_) => selectable_of(ri),
            Row::Header(_) => None,
        });
        match row {
            Row::New => {
                let style = if is_cursor {
                    Style::default().bg(Color::Blue).fg(Color::White)
                } else {
                    Style::default().fg(GREEN)
                };
                trows.push(
                    TRow::new(vec![
                        Cell::from(""),
                        Cell::from(Span::styled("+ New session", style)),
                        Cell::from(""),
                        Cell::from(""),
                    ])
                    .style(if is_cursor {
                        Style::default().bg(Color::Blue)
                    } else {
                        Style::default()
                    }),
                );
            }
            Row::Header(name) => {
                // Count sessions under this group for a subtle "(n)" suffix.
                let n = app.group_visible_count(ri);
                trows.push(TRow::new(vec![
                    Cell::from(""),
                    Cell::from(Line::from(vec![
                        Span::styled(
                            name.clone(),
                            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(format!("  ({n})"), Style::default().fg(DIM)),
                    ])),
                    Cell::from(""),
                    Cell::from(""),
                ]));
            }
            Row::Session(si) => {
                let s = &app.sessions[*si];
                let short = s.name.split_once('/').map(|x| x.1).unwrap_or(&s.name);
                let picked = app.is_picked(&s.name);

                let dot = if s.running { "●" } else { "○" };
                let dot_color = if s.running { GREEN } else { DIM };
                let name_color = if s.running { CYAN } else { DIM };
                let tick = if picked { "✓" } else { " " };
                // Tree connector nesting the session under its group header.
                let connector = if app.is_last_visible_in_group(ri) {
                    "└─"
                } else {
                    "├─"
                };

                // Marker cell: tick + gap + dot.
                let marker = Line::from(vec![
                    Span::styled(tick, Style::default().fg(YELLOW)),
                    Span::raw("  "),
                    Span::styled(dot, Style::default().fg(dot_color)),
                ]);

                let base = if is_cursor {
                    Style::default().bg(Color::Blue).fg(Color::White)
                } else if picked {
                    Style::default().bg(Color::Indexed(236))
                } else {
                    Style::default()
                };

                let name_style = if is_cursor {
                    base
                } else {
                    base.fg(name_color)
                };
                let ts_style = if is_cursor { base } else { base.fg(YELLOW) };
                // Connector is dim except on the cursor row (inherits base).
                let conn_style = if is_cursor { base } else { base.fg(DIM) };

                // Name cell: tree connector + session name.
                let name_cell = Line::from(vec![
                    Span::styled(connector, conn_style),
                    Span::raw(" "),
                    Span::styled(short.to_string(), name_style),
                ]);

                trows.push(
                    TRow::new(vec![
                        Cell::from(marker),
                        Cell::from(name_cell),
                        Cell::from(Span::styled(s.created.clone(), ts_style)),
                        Cell::from(Span::styled(s.activity.clone(), ts_style)),
                    ])
                    .style(base),
                );
            }
        }
    }

    let widths = [
        Constraint::Length(5),
        Constraint::Min(20),
        Constraint::Length(14),
        Constraint::Length(14),
    ];

    let header = TRow::new(vec![
        Cell::from(""),
        Cell::from(Span::styled("SESSION", Style::default().fg(DIM))),
        Cell::from(Span::styled("STARTED", Style::default().fg(DIM))),
        Cell::from(Span::styled("ACTIVE", Style::default().fg(DIM))),
    ]);

    let total_rows = trows.len();

    let table = Table::new(trows, widths)
        .header(header)
        .column_spacing(1)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(DIM)),
        );

    // Drive scrolling via TableState so the cursor row stays visible. We do
    // our own highlight styling above, so no highlight_style here.
    let mut state = TableState::default();
    state.select(cursor_visible_idx);
    f.render_stateful_widget(table, area, &mut state);

    // Record hit-test geometry. The table block borders (1 top) and the
    // one-line column header sit above the data rows, so data begins at
    // area.y + 2 and the visible data height is area.height - 3 (top border,
    // header, bottom border).
    let data_y = area.y.saturating_add(2);
    let data_h = area.height.saturating_sub(3);
    // Inside the left/right borders.
    let data_x = area.x.saturating_add(1);
    let data_w = area.width.saturating_sub(2);
    map.data_rect = Rect {
        x: data_x,
        y: data_y,
        width: data_w,
        height: data_h,
    };
    map.offset = state.offset();
    // Guard against the offset running past the row count.
    if map.offset > total_rows {
        map.offset = total_rows;
    }
    map.trow_to_selectable = trow_to_selectable;
    // Marker cell is the first table column (width 5), starting at data_x.
    map.marker_x_end = data_x.saturating_add(5);
}

fn render_action_bar(f: &mut Frame, area: Rect, ctx: &RenderCtx, map: &mut MouseMap) {
    let app = ctx.app;
    let nsel = app.picked.len();

    // Track the terminal column as we build spans, so each action segment gets
    // a clickable region for mouse hit-testing.
    map.action_regions.clear();
    map.action_bar_y = area.y;
    let mut col = area.x;

    let mut spans = vec![Span::raw("  ")];
    col += 2;
    let actions = [
        Action::Attach,
        Action::Rename,
        Action::Move,
        Action::Archive,
        Action::Delete,
    ];
    for (i, act) in actions.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("│", Style::default().fg(DIM)));
            col += 1;
        }
        // In the archived view, the archive action reads "unarchive".
        let verb = if *act == Action::Archive && app.show_archived {
            "unarchive"
        } else {
            act.label()
        };
        // Bulk-capable actions show the pick count when sessions are selected.
        let label = if nsel > 0
            && matches!(*act, Action::Delete | Action::Move | Action::Archive)
        {
            format!("  {verb} {nsel}  ")
        } else {
            format!("  {verb}  ")
        };
        let width = label.chars().count() as u16;
        let style = if *act == app.action {
            Style::default().bg(Color::Blue).fg(Color::White)
        } else {
            Style::default().fg(DIM)
        };
        map.action_regions.push((col, col + width, *act));
        col += width;
        spans.push(Span::styled(label, style));
    }

    if nsel > 0 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("✓ {nsel} selected"),
            Style::default().fg(YELLOW),
        ));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_hint(f: &mut Frame, area: Rect, ctx: &RenderCtx) {
    let app = ctx.app;
    let dim = Style::default().fg(DIM);

    let spans = if !app.search.is_empty() {
        vec![
            Span::raw("  "),
            Span::styled(app.search.clone(), Style::default().fg(CYAN)),
            Span::styled("█", Style::default().fg(CYAN)),
            Span::raw("    "),
            Span::styled("↵", dim),
            Span::raw(" run  "),
            Span::styled("⇥", dim),
            Span::raw(" action  "),
            Span::styled("␣", dim),
            Span::raw(" pick  "),
            Span::styled("esc", dim),
            Span::raw(" clear"),
        ]
    } else {
        let archive_hint = if app.show_archived {
            " active"
        } else {
            " archived"
        };
        vec![
            Span::raw("  "),
            Span::styled("↑↓/click", dim),
            Span::raw(" nav   type to search   "),
            Span::styled("↵", dim),
            Span::raw(" run  "),
            Span::styled("⇥", dim),
            Span::raw(" action  "),
            Span::styled("␣", dim),
            Span::raw(" pick  "),
            Span::styled("^A", dim),
            Span::raw(archive_hint),
            Span::raw("  "),
            Span::styled("q", dim),
            Span::raw(" quit"),
        ]
    };

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
