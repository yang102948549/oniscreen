//! Desktop-independent state, bounded ink storage and software rendering.
use crate::geometry::{Point, Rect, Viewport, WheelZoom};
use std::io::{Read, Write};

pub const MAX_PIXELS: usize = 16_777_216;
const MAX_POINTS: usize = 100_000;
const MAX_STROKES: usize = 2_000;
pub const INK: u32 = 0x00ff7040;

pub struct Frame {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u32>,
}
impl Frame {
    pub fn new(width: i32, height: i32) -> Result<Self, String> {
        let count = i64::from(width) * i64::from(height);
        if width <= 0 || height <= 0 || width > 8192 || height > 8192 || count > MAX_PIXELS as i64 {
            return Err("Unsupported frame dimensions (maximum 16 million pixels)".into());
        }
        Ok(Self {
            width,
            height,
            pixels: vec![0; count as usize],
        })
    }
    pub fn fixture(width: i32, height: i32) -> Result<Self, String> {
        let mut frame = Self::new(width, height)?;
        for y in 0..height {
            for x in 0..width {
                frame.pixels[(y * width + x) as usize] = if (x / 32 + y / 32) % 2 == 0 {
                    0x00e8eef6
                } else {
                    0x00384e70
                };
            }
        }
        Ok(frame)
    }
    pub fn write(&self, mut output: impl Write) -> Result<(), String> {
        output.write_all(b"SPF2").map_err(|e| e.to_string())?;
        output
            .write_all(&self.width.to_le_bytes())
            .map_err(|e| e.to_string())?;
        output
            .write_all(&self.height.to_le_bytes())
            .map_err(|e| e.to_string())?;
        for row in self.pixels.chunks(self.width as usize) {
            let bytes: Vec<u8> = row.iter().flat_map(|p| p.to_le_bytes()).collect();
            output.write_all(&bytes).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub fn read(mut input: impl Read) -> Result<Self, String> {
        let mut header = [0u8; 12];
        input.read_exact(&mut header).map_err(|e| e.to_string())?;
        if &header[..4] != b"SPF2" {
            return Err("Invalid capture protocol".into());
        }
        let mut frame = Self::new(
            i32::from_le_bytes(header[4..8].try_into().unwrap()),
            i32::from_le_bytes(header[8..12].try_into().unwrap()),
        )?;
        let mut row = vec![0; frame.width as usize * 4];
        for target in frame.pixels.chunks_mut(frame.width as usize) {
            input.read_exact(&mut row).map_err(|e| e.to_string())?;
            for (p, bytes) in target.iter_mut().zip(row.as_chunks::<4>().0) {
                *p = u32::from_le_bytes(*bytes);
            }
        }
        let mut extra = [0];
        if input.read(&mut extra).map_err(|e| e.to_string())? != 0 {
            return Err("Trailing capture data".into());
        }
        Ok(frame)
    }
    pub fn bmp(&self, mut output: impl Write) -> Result<(), String> {
        let size = self.pixels.len() as u32 * 4;
        let mut header = vec![0u8; 54];
        header[..2].copy_from_slice(b"BM");
        header[2..6].copy_from_slice(&(54 + size).to_le_bytes());
        header[10..14].copy_from_slice(&54u32.to_le_bytes());
        header[14..18].copy_from_slice(&40u32.to_le_bytes());
        header[18..22].copy_from_slice(&self.width.to_le_bytes());
        header[22..26].copy_from_slice(&(-self.height).to_le_bytes());
        header[26..28].copy_from_slice(&1u16.to_le_bytes());
        header[28..30].copy_from_slice(&32u16.to_le_bytes());
        header[34..38].copy_from_slice(&size.to_le_bytes());
        output.write_all(&header).map_err(|e| e.to_string())?;
        for p in &self.pixels {
            output
                .write_all(&p.to_le_bytes())
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

pub struct Canvas {
    pub view: Viewport,
    pub pen: bool,
    pub strokes: Vec<Vec<Point>>,
    drawing: bool,
    points: usize,
    wheel: WheelZoom,
}
impl Canvas {
    pub fn new(bounds: Rect) -> Self {
        let center = Point {
            x: bounds.left + bounds.width() / 2,
            y: bounds.top + bounds.height() / 2,
        };
        Self {
            view: Viewport::new(bounds, 2.0, center),
            pen: true,
            strokes: vec![],
            drawing: false,
            points: 0,
            wheel: WheelZoom::default(),
        }
    }
    pub fn zoom(&mut self, delta: f32, anchor: Point) {
        self.end();
        self.view.set_zoom(self.view.zoom + delta, anchor);
    }
    pub fn wheel(&mut self, delta: i32, anchor: Point) {
        let steps = self.wheel.steps(delta);
        self.zoom(steps as f32 * 0.25, anchor);
    }
    pub fn begin(&mut self, p: Point) {
        self.end();
        if !self.pen
            || !self.view.monitor.contains(p)
            || self.points >= MAX_POINTS
            || self.strokes.len() >= MAX_STROKES
        {
            return;
        }
        self.strokes.push(vec![self.view.to_source(p)]);
        self.points += 1;
        self.drawing = true;
    }
    pub fn extend(&mut self, p: Point) {
        // Leaving the client ends the stroke; never acquire global mouse capture.
        if !self.view.monitor.contains(p) || self.points >= MAX_POINTS {
            self.end();
        }
        if !self.drawing {
            return;
        }
        let p = self.view.to_source(p);
        if let Some(stroke) = self.strokes.last_mut() {
            if stroke.last() != Some(&p) {
                stroke.push(p);
                self.points += 1;
            }
        }
    }
    pub fn end(&mut self) {
        self.drawing = false;
    }
    pub fn clear(&mut self) {
        self.end();
        self.strokes.clear();
        self.points = 0;
    }
    pub fn undo(&mut self) {
        self.end();
        if let Some(stroke) = self.strokes.pop() {
            self.points -= stroke.len();
        }
    }
    pub fn screen_strokes(&self) -> Vec<Vec<Point>> {
        self.strokes
            .iter()
            .map(|stroke| {
                stroke
                    .iter()
                    .map(|p| Point {
                        x: ((p.x - self.view.source.left) as f32 * self.view.zoom).round() as i32,
                        y: ((p.y - self.view.source.top) as f32 * self.view.zoom).round() as i32,
                    })
                    .collect()
            })
            .collect()
    }
    pub fn render(&self, source: &Frame) -> Result<Frame, String> {
        let m = self.view.monitor;
        if source.width != m.width() || source.height != m.height() {
            return Err("Capture dimensions changed".into());
        }
        let mut out = Frame::new(source.width, source.height)?;
        for y in 0..out.height {
            for x in 0..out.width {
                let p = self.view.to_source(Point {
                    x: m.left + x,
                    y: m.top + y,
                });
                out.pixels[(y * out.width + x) as usize] =
                    source.pixels[((p.y - m.top) * source.width + p.x - m.left) as usize];
            }
        }
        self.ink(&mut out);
        Ok(out)
    }
    pub fn ink(&self, out: &mut Frame) {
        for stroke in self.screen_strokes() {
            if let Some(first) = stroke.first() {
                dot(out, *first);
            }
            for pair in stroke.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let steps = (b.x - a.x).abs().max((b.y - a.y).abs()).max(1);
                for i in 0..=steps {
                    dot(
                        out,
                        Point {
                            x: a.x + (b.x - a.x) * i / steps,
                            y: a.y + (b.y - a.y) * i / steps,
                        },
                    );
                }
            }
        }
    }
}
fn dot(frame: &mut Frame, p: Point) {
    for y in (p.y - 2).max(0)..=(p.y + 2).min(frame.height - 1) {
        for x in (p.x - 2).max(0)..=(p.x + 2).min(frame.width - 1) {
            frame.pixels[(y * frame.width + x) as usize] = INK;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bounds() -> Rect {
        Rect {
            left: -100,
            top: -40,
            right: 28,
            bottom: 56,
        }
    }
    #[test]
    fn zoom_uses_source_pixels_and_ink_survives_pan_zoom() {
        let mut c = Canvas::new(bounds());
        let f = Frame::fixture(128, 96).unwrap();
        let p = Point { x: -36, y: 8 };
        c.begin(p);
        c.extend(Point { x: -16, y: 18 });
        c.end();
        let saved = c.strokes.clone();
        c.zoom(0.5, p);
        c.view.pan(5, 2);
        assert_eq!(c.strokes, saved);
        let rendered = c.render(&f).unwrap();
        assert!(rendered.pixels.contains(&INK));
        c.clear();
        let rendered = c.render(&f).unwrap();
        let source = c.view.to_source(Point {
            x: bounds().left,
            y: bounds().top,
        });
        assert_eq!(
            rendered.pixels[0],
            f.pixels[((source.y + 40) * 128 + source.x + 100) as usize]
        );
    }
    #[test]
    fn leaving_or_losing_button_ends_stroke_and_undo_clears_dot() {
        let mut c = Canvas::new(bounds());
        c.begin(Point { x: -36, y: 8 });
        c.extend(Point { x: 999, y: 999 });
        c.extend(Point { x: -20, y: 8 });
        assert_eq!(c.strokes[0].len(), 1);
        c.undo();
        assert!(c.strokes.is_empty());
        c.pen = false;
        c.begin(Point { x: -36, y: 8 });
        assert!(c.strokes.is_empty());
    }
    #[test]
    fn frame_protocol_rejects_truncation_oversize_and_trailing_bytes() {
        let f = Frame::fixture(8, 8).unwrap();
        let mut bytes = vec![];
        f.write(&mut bytes).unwrap();
        assert_eq!(Frame::read(&bytes[..]).unwrap().pixels, f.pixels);
        assert!(Frame::read(&bytes[..20]).is_err());
        bytes.push(1);
        assert!(Frame::read(&bytes[..]).is_err());
        assert!(Frame::new(i32::MAX, i32::MAX).is_err());
        assert!(Frame::new(0, 3).is_err());
    }
    #[test]
    fn ink_limits_bound_memory() {
        let mut c = Canvas::new(bounds());
        for _ in 0..MAX_STROKES + 20 {
            c.begin(Point { x: -36, y: 8 });
        }
        assert_eq!(c.strokes.len(), MAX_STROKES);
        c.clear();
        assert_eq!(c.points, 0);
    }
}
