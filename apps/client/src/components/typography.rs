//! Text roles. Each sets size, line height and weight in points; set colour separately.
//! Views use these instead of `text_xs/sm/...` (rem-based) or ad-hoc `text_size`.
//! Roles inherit, so applying one to a wrapper styles Kit `Input`/`TextView` children too.

use std::sync::Arc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

fn role<T: Styled>(el: T, size: f32, line: f32, weight: FontWeight) -> T {
    el.text_size(px(size)).line_height(px(line)).font_weight(weight)
}

/// Tabular figures, for counts and stat values that update in place.
pub fn tabular() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}

pub trait Typography: Styled + Sized {
    /// Page title, 22/28 semibold.
    fn t_title(self) -> Self {
        role(self, 22., 28., FontWeight::SEMIBOLD)
    }
    /// Review tour/page title, 20/26 semibold.
    fn t_read_title(self) -> Self {
        role(self, 20., 26., FontWeight::SEMIBOLD)
    }
    /// Reading section heading (review chapter, "Overview"), 17/24 semibold.
    fn t_read_heading(self) -> Self {
        role(self, 17., 24., FontWeight::SEMIBOLD)
    }
    /// Compact drawer/dialog title, 17/22 semibold.
    fn t_headline(self) -> Self {
        role(self, 17., 22., FontWeight::SEMIBOLD)
    }
    /// In-page group heading, 15/20 semibold.
    fn t_section(self) -> Self {
        role(self, 15., 20., FontWeight::SEMIBOLD)
    }
    /// Descriptions, subtitles, settings rows, empty-state copy, 14/20.
    fn t_body(self) -> Self {
        role(self, 14., 20., FontWeight::NORMAL)
    }
    /// Guide/overview prose, 14/22.
    fn t_read(self) -> Self {
        role(self, 14., 22., FontWeight::NORMAL)
    }
    /// UI default: nav, tabs, rows, buttons, inputs, values, 13/18.
    fn t_label(self) -> Self {
        role(self, 13., 18., FontWeight::NORMAL)
    }
    /// Trace span names only, 16/22 (measured on F263F6D3 38pt rows).
    fn t_trace_label(self) -> Self {
        role(self, 16., 22., FontWeight::NORMAL)
    }
    /// Panel/popover title: row size, semibold, 13/18.
    fn t_panel_title(self) -> Self {
        role(self, 13., 18., FontWeight::SEMIBOLD)
    }
    /// Meta, counts, chips, timestamps, 12/16.
    fn t_caption(self) -> Self {
        role(self, 12., 16., FontWeight::NORMAL)
    }
    /// Stat-strip labels, heatmap axes and kbd hints only, 11/14 medium.
    fn t_micro(self) -> Self {
        role(self, 11., 14., FontWeight::MEDIUM)
    }
    /// Document H1, 28/34 semibold.
    fn t_doc_title(self) -> Self {
        role(self, 28., 34., FontWeight::SEMIBOLD)
    }
    /// Document H2, 19/26 semibold.
    fn t_doc_h2(self) -> Self {
        role(self, 19., 26., FontWeight::SEMIBOLD)
    }
    /// Document H3, 16/22 semibold.
    fn t_doc_h3(self) -> Self {
        role(self, 16., 22., FontWeight::SEMIBOLD)
    }
    /// Document paragraphs and lists, 15/24 (measure ≤ 680).
    fn t_prose(self) -> Self {
        role(self, 15., 24., FontWeight::NORMAL)
    }
    /// Large figures ("1,705", "$20"), 28/32 semibold tabular.
    fn t_display(self) -> Self {
        role(self, 28., 32., FontWeight::SEMIBOLD).font_features(tabular())
    }
    /// Stat tile value, 22/28 semibold tabular.
    fn t_stat(self) -> Self {
        role(self, 22., 28., FontWeight::SEMIBOLD).font_features(tabular())
    }
    /// Metric-strip value, 20/26 medium tabular.
    fn t_stat_sm(self) -> Self {
        role(self, 20., 26., FontWeight::MEDIUM).font_features(tabular())
    }
    /// Diff and code lines, 12.5/20 mono.
    fn t_code(self, cx: &App) -> Self {
        role(self, 12.5, 20., FontWeight::NORMAL).font_family(cx.theme().mono_font_family.clone())
    }
    /// Mono ids and number boxes, 12/16.
    fn t_code_caption(self, cx: &App) -> Self {
        role(self, 12., 16., FontWeight::NORMAL).font_family(cx.theme().mono_font_family.clone())
    }
}

impl<T: Styled> Typography for T {}

/// Markdown `TextView` style matching the document roles: H1 28, H2 19, H3 16, other
/// headings at prose size. Wrap the view in `t_prose()` (or `t_read()`) for body text;
/// Kit sizes headings from this function, not from inherited text style.
pub fn doc_text_view_style() -> gpui_kit::component::text::TextViewStyle {
    gpui_kit::component::text::TextViewStyle::default()
        .paragraph_gap(rems(0.75))
        .heading_font_size(|level, _| match level {
            1 => px(28.),
            2 => px(19.),
            3 => px(16.),
            _ => px(15.),
        })
}
