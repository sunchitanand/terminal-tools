//! ratatui rendering. Keeps the tabular layout of the zsh version: a table
//! with SESSION / STARTED / ACTIVE columns, project group headers, a running
//! dot, a bulk-select tick, plus a search line and an action bar.

use crate::app::{Action, App, Row};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row as TRow, Table, TableState};

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
}

fn render_title(f: &mut Frame, area: Rect, ctx: &RenderCtx) {
    let line = Line::from(vec![
        Span::raw("  "),
        Span::styled("⚡ tmux :: ", Style::default().fg(ACCENT)),
        Span::styled(
            ctx.host_short.to_string(),
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        ),
    ]);
    f.render_widget(Paragraph::new(line), area);
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
    for (i, act) in [Action::Attach, Action::Rename, Action::Delete]
        .iter()
        .enumerate()
    {
        if i > 0 {
            spans.push(Span::styled("│", Style::default().fg(DIM)));
            col += 1;
        }
        let label = if *act == Action::Delete && nsel > 0 {
            format!("  delete {nsel}  ")
        } else {
            format!("  {}  ", act.label())
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
            Span::styled("q", dim),
            Span::raw(" quit"),
        ]
    };

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
