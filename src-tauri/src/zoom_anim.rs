//! 확대 배율 애니메이션과 배율 사다리. PowerToys ZoomIt(MIT)의 `ZoomAnimation.cpp`와
//! `Zoomit.cpp`의 휠 확대 로직(telescoping)을 Rust로 옮긴 순수 계산 모듈이다. OS 호출이 없다.
//!
//! - 배율은 로그 공간에서 에르미트 곡선으로 보간하여, 2→4→8처럼 배율이 곱셈으로 변해도 체감 속도가 일정하다.
//! - 애니메이션 도중 다시 목표를 바꾸면(`retarget`) 현재 속도를 이어받아 끊김 없이 방향을 바꾼다.

/// 한 단계 배율 변화에 걸리는 시간 계산 (ZoomIt `ZoomAnimation::Duration`).
pub fn duration_ms(current: f32, target: f32, first_step_immediate: bool) -> u64 {
    const STEP_MS: u64 = 40;
    if current <= 0.0 || target <= 0.0 || current == target {
        return 0;
    }
    let (mut z, mut steps) = (current, 0u64);
    if target > current {
        while z < target {
            z *= 1.1;
            steps += 1;
        }
    } else {
        while z > target {
            z *= 0.8;
            steps += 1;
        }
    }
    if first_step_immediate && steps != 0 {
        steps -= 1;
    }
    steps * STEP_MS * 3 / 2
}

#[derive(Debug, Clone)]
pub struct ZoomAnim {
    start: f32,
    current: f32,
    target: f32,
    start_ms: u64,
    dur_ms: u64,
    /// 시작 속도 (ln 배율 / ms)
    v0: f32,
    active: bool,
}

impl ZoomAnim {
    pub fn new(zoom: f32) -> Self {
        Self { start: zoom, current: zoom, target: zoom, start_ms: 0, dur_ms: 0, v0: 0.0, active: false }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn stop(&mut self, zoom: f32) {
        *self = Self::new(zoom);
    }

    pub fn start(&mut self, current: f32, target: f32, now_ms: u64, dur_ms: u64) {
        self.start = current;
        self.current = current;
        self.target = target;
        self.start_ms = now_ms;
        self.dur_ms = dur_ms;
        self.v0 = 0.0;
        self.active = current > 0.0 && target > 0.0 && current != target && dur_ms != 0;
        if !self.active {
            self.current = target;
        }
    }

    /// 진행 중인 애니메이션의 속도를 이어받아 새 목표로 전환한다. 새 현재 배율을 돌려준다.
    pub fn retarget(&mut self, target: f32, now_ms: u64, dur_ms: u64) -> f32 {
        let mut v = 0.0f32;
        if self.active && self.dur_ms != 0 {
            let elapsed = now_ms.saturating_sub(self.start_ms) as f32;
            let p = (elapsed / self.dur_ms as f32).min(1.0);
            let (s0, s1, d) = (self.start.ln(), self.target.ln(), self.dur_ms as f32);
            v = ((6.0 * p * p - 6.0 * p) * s0
                + (3.0 * p * p - 4.0 * p + 1.0) * d * self.v0
                + (-6.0 * p * p + 6.0 * p) * s1)
                / d;
        }
        let cur = self.sample(now_ms);
        self.start(cur, target, now_ms, dur_ms);
        if !self.active {
            return self.current;
        }
        let dist = target.ln() - cur.ln();
        if v * dist > 0.0 {
            let max_v = 3.0 * dist.abs() / dur_ms as f32;
            self.v0 = v.clamp(-max_v, max_v);
        }
        self.current
    }

    pub fn sample(&mut self, now_ms: u64) -> f32 {
        if !self.active {
            return self.current;
        }
        let elapsed = now_ms.saturating_sub(self.start_ms);
        if elapsed >= self.dur_ms {
            self.current = self.target;
            self.active = false;
            return self.current;
        }
        let p = elapsed as f32 / self.dur_ms as f32;
        let (p2, p3) = (p * p, p * p * p);
        let w_start = 2.0 * p3 - 3.0 * p2 + 1.0;
        let w_vel = p3 - 2.0 * p2 + p;
        let w_target = -2.0 * p3 + 3.0 * p2;
        let scale = w_start * self.start.ln() + w_vel * self.dur_ms as f32 * self.v0 + w_target * self.target.ln();
        self.current = scale.exp();
        self.current
    }
}

/// ZoomIt 휠 확대 사다리: 1 → 2 → 4 → 8 …, 2배 미만 구간은 ×0.75로 천천히 1배로 내려온다.
pub fn ladder_in(target: f32, max: f32) -> f32 {
    if target >= max {
        max
    } else if target < 2.0 {
        2.0f32.min(max)
    } else {
        (target * 2.0).min(max)
    }
}

/// 사다리 한 칸 축소. 결과가 1 이하이면 확대 종료를 뜻한다.
pub fn ladder_out(target: f32) -> f32 {
    if target <= 2.0 {
        // ZoomIt은 1.125배까지 내려가지만, 1.2배 미만은 의미 없는 배율이라 바로 종료(1배)로 처리한다.
        let t = target * 0.75;
        if t < 1.2 {
            1.0
        } else {
            t
        }
    } else {
        target / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_matches_zoomit_steps() {
        assert_eq!(duration_ms(2.0, 2.0, true), 0);
        // 1→2: ×1.1 을 8번(1.1^8≈2.14) 곱해야 넘는다. 첫 단계는 즉시 → 7단계 × 40ms × 1.5
        assert_eq!(duration_ms(1.0, 2.0, true), 7 * 40 * 3 / 2);
        // 축소는 ×0.8 : 4→2 는 4*0.8^4=1.64 → 4단계
        assert_eq!(duration_ms(4.0, 2.0, false), 4 * 40 * 3 / 2);
    }

    #[test]
    fn animation_reaches_target_monotonically() {
        let mut a = ZoomAnim::new(1.0);
        a.start(1.0, 4.0, 1000, 300);
        let mut prev = 1.0;
        for t in (1000..=1300).step_by(10) {
            let z = a.sample(t);
            assert!(z >= prev - 1e-4, "배율은 감소하면 안 된다: {prev} -> {z}");
            prev = z;
        }
        assert!((a.sample(1300) - 4.0).abs() < 1e-4);
        assert!(!a.is_active());
    }

    #[test]
    fn retarget_is_continuous() {
        let mut a = ZoomAnim::new(1.0);
        a.start(1.0, 8.0, 0, 400);
        let mid = a.sample(200);
        let after = a.retarget(2.0, 200, 300);
        assert!((after - mid).abs() < 1e-3, "방향을 바꿔도 배율이 튀면 안 된다");
        assert!((a.sample(500) - 2.0).abs() < 1e-3);
    }

    #[test]
    fn zero_duration_jumps() {
        let mut a = ZoomAnim::new(1.0);
        a.start(1.0, 3.0, 0, 0);
        assert!(!a.is_active());
        assert_eq!(a.sample(0), 3.0);
    }

    #[test]
    fn ladder_follows_zoomit() {
        assert_eq!(ladder_in(1.0, 8.0), 2.0);
        assert_eq!(ladder_in(2.0, 8.0), 4.0);
        assert_eq!(ladder_in(8.0, 8.0), 8.0);
        assert_eq!(ladder_out(8.0), 4.0);
        assert_eq!(ladder_out(2.0), 1.5);
        assert_eq!(ladder_out(1.5), 1.0);
    }
}
