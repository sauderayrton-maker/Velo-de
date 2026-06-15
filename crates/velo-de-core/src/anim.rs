//! A tiny spring-damper animation primitive.
//!
//! Everything that moves in Velo-de (strip scrolling, the camera pan
//! between Spaces, the Overview zoom level) is driven by one of these: set
//! a target and call [`Animated::tick`] once per frame. The value glides
//! toward the target with critically-damped spring physics instead of a
//! linear/eased tween, which is what gives the workspace transitions their
//! "physical" feel.

/// Types that can be animated with spring physics: addition, scaling by a
/// scalar, and a magnitude used to decide when the spring has settled.
pub trait Animatable: Copy {
    const ZERO: Self;
    fn add(self, other: Self) -> Self;
    fn sub(self, other: Self) -> Self;
    fn scale(self, factor: f64) -> Self;
    /// A non-negative measure of size, used for the settle threshold.
    fn magnitude(self) -> f64;
}

impl Animatable for f64 {
    const ZERO: f64 = 0.0;
    fn add(self, other: f64) -> f64 {
        self + other
    }
    fn sub(self, other: f64) -> f64 {
        self - other
    }
    fn scale(self, factor: f64) -> f64 {
        self * factor
    }
    fn magnitude(self) -> f64 {
        self.abs()
    }
}

impl Animatable for super::geometry::Vec2 {
    const ZERO: Self = super::geometry::Vec2::ZERO;
    fn add(self, other: Self) -> Self {
        self + other
    }
    fn sub(self, other: Self) -> Self {
        self - other
    }
    fn scale(self, factor: f64) -> Self {
        self * factor
    }
    fn magnitude(self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
}

/// Stiffness/damping tuning for spring animations. Higher stiffness snaps
/// faster; higher damping reduces overshoot. The defaults are
/// critically-damped-ish for a quick, non-bouncy "slide".
#[derive(Debug, Clone, Copy)]
pub struct SpringParams {
    pub stiffness: f64,
    pub damping: f64,
}

impl Default for SpringParams {
    fn default() -> Self {
        Self { stiffness: 220.0, damping: 26.0 }
    }
}

/// A value that springs toward a target over time.
#[derive(Debug, Clone, Copy)]
pub struct Animated<T: Animatable> {
    value: T,
    velocity: T,
    target: T,
    params: SpringParams,
}

/// Below this combined value+velocity delta, an animation is considered
/// settled and snaps exactly onto its target.
const SETTLE_EPSILON: f64 = 0.01;

impl<T: Animatable> Animated<T> {
    pub fn new(value: T) -> Self {
        Self { value, velocity: T::ZERO, target: value, params: SpringParams::default() }
    }

    pub fn with_params(value: T, params: SpringParams) -> Self {
        Self { value, velocity: T::ZERO, target: value, params }
    }

    pub fn value(&self) -> T {
        self.value
    }

    pub fn target(&self) -> T {
        self.target
    }

    pub fn set_target(&mut self, target: T) {
        self.target = target;
    }

    /// Immediately jump to a value with no animation (e.g. on first layout).
    pub fn jump_to(&mut self, value: T) {
        self.value = value;
        self.target = value;
        self.velocity = T::ZERO;
    }

    /// True once the value has reached (and stopped moving toward) its
    /// target. Drives whether the compositor needs to keep redrawing.
    pub fn is_settled(&self) -> bool {
        self.value.sub(self.target).magnitude() < SETTLE_EPSILON && self.velocity.magnitude() < SETTLE_EPSILON
    }

    /// Advance the spring by `dt` seconds.
    pub fn tick(&mut self, dt: f64) {
        if self.is_settled() {
            self.value = self.target;
            self.velocity = T::ZERO;
            return;
        }

        let displacement = self.value.sub(self.target);
        let accel = displacement.scale(-self.params.stiffness).sub(self.velocity.scale(self.params.damping));
        self.velocity = self.velocity.add(accel.scale(dt));
        self.value = self.value.add(self.velocity.scale(dt));

        if self.is_settled() {
            self.value = self.target;
            self.velocity = T::ZERO;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settles_at_target() {
        let mut a = Animated::new(0.0_f64);
        a.set_target(100.0);
        assert!(!a.is_settled());

        for _ in 0..1000 {
            a.tick(1.0 / 60.0);
            if a.is_settled() {
                break;
            }
        }

        assert!(a.is_settled());
        assert!((a.value() - 100.0).abs() < 1e-6);
    }

    #[test]
    fn jump_to_is_immediate() {
        let mut a = Animated::new(0.0_f64);
        a.set_target(50.0);
        a.tick(1.0 / 60.0);
        assert!(!a.is_settled());

        a.jump_to(10.0);
        assert_eq!(a.value(), 10.0);
        assert_eq!(a.target(), 10.0);
        assert!(a.is_settled());
    }

    #[test]
    fn approaches_monotonically_for_critically_damped_params() {
        let params = SpringParams { stiffness: 220.0, damping: 2.0 * 220.0_f64.sqrt() };
        let mut a = Animated::with_params(0.0_f64, params);
        a.set_target(10.0);

        let mut last = a.value();
        for _ in 0..600 {
            a.tick(1.0 / 120.0);
            let v = a.value();
            assert!(v >= last - 1e-9, "value should not overshoot/oscillate back down");
            assert!(v <= 10.0 + 1e-6, "value should not overshoot target");
            last = v;
            if a.is_settled() {
                break;
            }
        }
        assert!((last - 10.0).abs() < 1e-3);
    }
}
