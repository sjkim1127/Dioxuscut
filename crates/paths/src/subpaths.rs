//! SVG subpath extraction.

use crate::parser::{parse_path, serialize_instructions};
use crate::types::Instruction;

/// Splits a compound SVG path into individual subpaths based on `M` (move) commands.
///
/// Returns an empty vector if the path cannot be parsed or contains no instructions.
pub fn get_subpaths(path: &str) -> Vec<String> {
    let instructions = match parse_path(path) {
        Ok(insts) => insts,
        Err(_) => return Vec::new(),
    };

    let mut subpaths = Vec::new();
    let mut current_subpath = Vec::new();

    for inst in instructions {
        if matches!(inst, Instruction::MoveTo { .. }) && !current_subpath.is_empty() {
            subpaths.push(serialize_instructions(&current_subpath));
            current_subpath.clear();
        }
        current_subpath.push(inst);
    }

    if !current_subpath.is_empty() {
        subpaths.push(serialize_instructions(&current_subpath));
    }

    subpaths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_subpaths() {
        let path = "M 0 0 L 10 10 M 20 20 L 30 30 Z";
        let subpaths = get_subpaths(path);
        assert_eq!(subpaths.len(), 2);
        assert_eq!(subpaths[0], "M 0.0000 0.0000 L 10.0000 10.0000");
        assert_eq!(subpaths[1], "M 20.0000 20.0000 L 30.0000 30.0000 Z");
    }
}
