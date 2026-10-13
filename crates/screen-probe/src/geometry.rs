//! All engine coordinates are physical desktop pixels, including negative origins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// Accumulate partial notches from precision wheels without losing direction.
#[derive(Default)]
pub struct WheelZoom(i32);
impl WheelZoom {
    pub fn steps(&mut self, delta: i32) -> i32 {
        self.0 = self.0.saturating_add(delta);
        let steps = self.0 / 120;
        self.0 %= 120;
        steps
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(self) -> i32 {
        self.right - self.left
    }
    pub fn height(self) -> i32 {
        self.bottom - self.top
    }
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.left && p.x < self.right && p.y >= self.top && p.y < self.bottom
    }
    pub fn clamp(self, p: Point) -> Point {
        Point {
            x: p.x.clamp(self.left, self.right - 1),
            y: p.y.clamp(self.top, self.bottom - 1),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub monitor: Rect,
    pub source: Rect,
    pub zoom: f32,
}

impl Viewport {
    pub fn new(monitor: Rect, zoom: f32, anchor: Point) -> Self {
        let mut v = Self {
            monitor,
            source: monitor,
            zoom: 1.0,
        };
        v.set_zoom(zoom, anchor);
        v
    }
    pub fn set_zoom(&mut self, zoom: f32, anchor: Point) {
        let anchor = self.monitor.clamp(anchor);
        let source_anchor = self.to_source(anchor);
        self.zoom = if zoom.is_finite() {
            zoom.clamp(1.0, 4.0)
        } else {
            1.0
        };
        let width = (self.monitor.width() as f32 / self.zoom).ceil() as i32;
        let height = (self.monitor.height() as f32 / self.zoom).ceil() as i32;
        let left =
            source_anchor.x - ((anchor.x - self.monitor.left) as f32 / self.zoom).round() as i32;
        let top =
            source_anchor.y - ((anchor.y - self.monitor.top) as f32 / self.zoom).round() as i32;
        let left = left.clamp(self.monitor.left, self.monitor.right - width);
        let top = top.clamp(self.monitor.top, self.monitor.bottom - height);
        self.source = Rect {
            left,
            top,
            right: left + width,
            bottom: top + height,
        };
    }
    pub fn to_source(self, p: Point) -> Point {
        self.source.clamp(Point {
            x: self.source.left + ((p.x - self.monitor.left) as f32 / self.zoom).round() as i32,
            y: self.source.top + ((p.y - self.monitor.top) as f32 / self.zoom).round() as i32,
        })
    }
    pub fn pan(&mut self, dx: i32, dy: i32) {
        let width = self.source.width();
        let height = self.source.height();
        self.source.left = self
            .source
            .left
            .saturating_add(dx)
            .clamp(self.monitor.left, self.monitor.right - width);
        self.source.top = self
            .source
            .top
            .saturating_add(dy)
            .clamp(self.monitor.top, self.monitor.bottom - height);
        self.source.right = self.source.left + width;
        self.source.bottom = self.source.top + height;
    }
}

/// SendInput's absolute coordinates cover the complete virtual desktop, not the primary display.
#[cfg(test)]
pub fn absolute(p: Point, desktop: Rect) -> (i32, i32) {
    let p = desktop.clamp(p);
    // Target the middle of the pixel's absolute-coordinate interval.
    let axis = |position: i32, start: i32, extent: i32| -> i32 {
        (((i64::from(position - start) * 65536 + 32768) / i64::from(extent)).clamp(0, 65535)) as i32
    };
    (
        axis(p.x, desktop.left, desktop.width()),
        axis(p.y, desktop.top, desktop.height()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wheel_accumulates_partial_notches_and_reverses() {
        let mut wheel = WheelZoom::default();
        assert_eq!(wheel.steps(60), 0);
        assert_eq!(wheel.steps(60), 1);
        assert_eq!(wheel.steps(-240), -2);
        assert_eq!(wheel.steps(60), 0);
        assert_eq!(wheel.steps(-60), 0);
        assert_eq!(wheel.steps(-120), -1);
    }
    const MON: Rect = Rect {
        left: -1920,
        top: -200,
        right: 0,
        bottom: 880,
    };
    #[test]
    fn negative_monitor_anchor_is_stable() {
        let p = Point { x: -1000, y: 300 };
        let v = Viewport::new(MON, 2.0, p);
        let result = v.to_source(p);
        assert!((result.x - p.x).abs() <= 1 && (result.y - p.y).abs() <= 1);
    }
    #[test]
    fn pan_stays_on_selected_monitor() {
        let mut v = Viewport::new(MON, 4.0, Point { x: -900, y: 200 });
        v.pan(i32::MAX / 2, -100000);
        assert_eq!(v.source.right, MON.right);
        assert_eq!(v.source.top, MON.top);
        assert_eq!(v.source.width(), 480);
    }
    #[test]
    fn zoom_limits_and_identity() {
        let mut v = Viewport::new(MON, 10.0, Point { x: -1200, y: 100 });
        assert_eq!(v.zoom, 4.0);
        v.set_zoom(f32::NAN, Point::default());
        assert_eq!(v.source, MON);
        assert_eq!(
            v.to_source(Point { x: -30, y: 25 }),
            Point { x: -30, y: 25 }
        );
    }
    #[test]
    fn every_screen_corner_maps_inside_source() {
        for zoom in [1.0, 1.25, 1.5, 2.0, 3.75, 4.0] {
            let v = Viewport::new(MON, zoom, Point { x: -1, y: 879 });
            for p in [Point { x: -1920, y: -200 }, Point { x: -1, y: 879 }] {
                assert!(v.source.contains(v.to_source(p)));
            }
            assert!(v.source.left >= MON.left && v.source.right <= MON.right);
        }
    }
    #[test]
    fn absolute_input_round_trips_negative_virtual_desktop() {
        let d = Rect {
            left: -1920,
            top: -200,
            right: 3840,
            bottom: 2160,
        };
        for p in [
            Point { x: -1920, y: -200 },
            Point { x: -1, y: 0 },
            Point { x: 3839, y: 2159 },
        ] {
            let (x, y) = absolute(p, d);
            assert_eq!((x as i64 * d.width() as i64 / 65536) as i32 + d.left, p.x);
            assert_eq!((y as i64 * d.height() as i64 / 65536) as i32 + d.top, p.y);
        }
    }
}
