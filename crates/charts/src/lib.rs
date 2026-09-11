pub mod d3;
pub mod mermaid;
pub mod quickjs_runtime;
pub mod svg_parser;

pub use d3::{CurveType, D3BarChart, D3LineChart, D3PieChart};
pub use mermaid::{render_mermaid, render_mermaid_svg, DiagramTheme, Direction, NodeShape};
pub use quickjs_runtime::{ChartError, QuickJsEngine};
pub use svg_parser::{parse_svg, SvgDocument};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
