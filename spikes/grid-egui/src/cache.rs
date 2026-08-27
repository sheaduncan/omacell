//! Shaping cache keyed by `(text, style, zoom)` as in spec §11.4.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, FontId, Galley};

const CAP: usize = 8192;

#[derive(Clone, Hash, Eq, PartialEq)]
struct Key {
    text: String,
    size_milli: u16,
    ppp_milli: u16,
    color: u32,
}

pub struct ShapeCache {
    map: HashMap<Key, Arc<Galley>>,
}

impl ShapeCache {
    pub fn new() -> Self {
        Self {
            map: HashMap::with_capacity(1024),
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn galley(
        &mut self,
        ctx: &egui::Context,
        text: String,
        font: FontId,
        color: Color32,
        ppp: f32,
    ) -> Arc<Galley> {
        let key = Key {
            text: text.clone(),
            size_milli: (font.size * 1000.0) as u16,
            ppp_milli: (ppp * 1000.0) as u16,
            color: color
                .to_array()
                .into_iter()
                .fold(0u32, |a, b| (a << 8) | u32::from(b)),
        };
        if let Some(g) = self.map.get(&key) {
            return g.clone();
        }
        if self.map.len() >= CAP {
            self.map.clear();
        }
        let galley = ctx.fonts_mut(|f| f.layout_no_wrap(text, font, color));
        self.map.insert(key, galley.clone());
        galley
    }
}
