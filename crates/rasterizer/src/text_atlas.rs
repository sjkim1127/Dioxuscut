//! CPU-side packed text coverage atlas shared by native and future GPU paths.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AtlasEntry {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub baseline: u32,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct TextAtlasSnapshot {
    pub width: u32,
    pub height: u32,
    pub generation: u64,
    pub pixels: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct TextAtlas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
    entries: HashMap<String, AtlasEntry>,
    generation: u64,
}

impl TextAtlas {
    pub(crate) fn new(width: u32, height: u32) -> Self {
        assert!(width > 0 && height > 0);
        Self {
            width,
            height,
            pixels: vec![0; (width * height) as usize],
            cursor_x: 0,
            cursor_y: 0,
            row_height: 0,
            entries: HashMap::new(),
            generation: 0,
        }
    }

    pub(crate) fn insert(
        &mut self,
        key: String,
        coverage: &[u8],
        glyph_width: u32,
        glyph_height: u32,
        baseline: u32,
    ) -> Option<AtlasEntry> {
        if let Some(entry) = self.entries.get(&key).copied() {
            return Some(entry);
        }
        if glyph_width == 0 || glyph_height == 0 || coverage.len() != (glyph_width * glyph_height) as usize
            || glyph_width > self.width || glyph_height > self.height
        {
            return None;
        }
        if self.cursor_x + glyph_width > self.width {
            self.cursor_x = 0;
            self.cursor_y = self.cursor_y.saturating_add(self.row_height);
            self.row_height = 0;
        }
        if self.cursor_y + glyph_height > self.height {
            return None;
        }
        let entry = AtlasEntry {
            x: self.cursor_x,
            y: self.cursor_y,
            width: glyph_width,
            height: glyph_height,
            baseline,
        };
        for row in 0..glyph_height {
            let source = &coverage[(row * glyph_width) as usize..((row + 1) * glyph_width) as usize];
            let target_start = ((entry.y + row) * self.width + entry.x) as usize;
            self.pixels[target_start..target_start + glyph_width as usize].copy_from_slice(source);
        }
        self.cursor_x = self.cursor_x.saturating_add(glyph_width);
        self.row_height = self.row_height.max(glyph_height);
        self.entries.insert(key, entry);
        self.generation = self.generation.wrapping_add(1);
        Some(entry)
    }

    #[allow(dead_code)]
    pub(crate) fn entry(&self, key: &str) -> Option<AtlasEntry> {
        self.entries.get(key).copied()
    }

    pub(crate) fn clear(&mut self) {
        self.pixels.fill(0);
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.row_height = 0;
        self.entries.clear();
        self.generation = self.generation.wrapping_add(1);
    }

    #[allow(dead_code)]
    pub(crate) fn snapshot(&self) -> TextAtlasSnapshot {
        TextAtlasSnapshot {
            width: self.width,
            height: self.height,
            generation: self.generation,
            pixels: self.pixels.clone(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn sample(&self, entry: AtlasEntry) -> Vec<u8> {
        let mut output = vec![0; (entry.width * entry.height) as usize];
        for row in 0..entry.height {
            let source_start = ((entry.y + row) * self.width + entry.x) as usize;
            let target_start = (row * entry.width) as usize;
            output[target_start..target_start + entry.width as usize]
                .copy_from_slice(&self.pixels[source_start..source_start + entry.width as usize]);
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_entries_and_preserves_pixels() {
        let mut atlas = TextAtlas::new(4, 4);
        let first = atlas.insert("a".into(), &[1, 2, 3, 4], 2, 2, 1).unwrap();
        let second = atlas.insert("b".into(), &[5, 6], 2, 1, 0).unwrap();
        assert_eq!(first, AtlasEntry { x: 0, y: 0, width: 2, height: 2, baseline: 1 });
        assert_eq!(second.x, 2);
        assert_eq!(atlas.sample(first), vec![1, 2, 3, 4]);
        assert_eq!(atlas.sample(second), vec![5, 6]);
        assert_eq!(atlas.entry("a"), Some(first));
    }

    #[test]
    fn rejects_invalid_or_overflowing_entries() {
        let mut atlas = TextAtlas::new(2, 2);
        assert!(atlas.insert("bad".into(), &[1], 2, 2, 0).is_none());
        assert!(atlas.insert("a".into(), &[1, 2, 3, 4], 2, 2, 0).is_some());
        assert!(atlas.insert("b".into(), &[1], 1, 1, 0).is_none());
    }

    #[test]
    fn snapshot_is_stable_and_clear_resets_storage() {
        let mut atlas = TextAtlas::new(4, 2);
        atlas.insert("text".into(), &[9, 8], 2, 1, 1).unwrap();
        let snapshot = atlas.snapshot();
        assert_eq!((snapshot.width, snapshot.height), (4, 2));
        assert_eq!(&snapshot.pixels[..2], &[9, 8]);

        atlas.clear();
        assert!(atlas.entry("text").is_none());
        assert!(atlas.snapshot().pixels.iter().all(|pixel| *pixel == 0));
        assert_eq!(&snapshot.pixels[..2], &[9, 8]);
    }
}
