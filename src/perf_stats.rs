use std::time::Instant;

/// Exponential moving average smoother.
///
/// Alpha of 0.05 gives roughly a 20-sample smoothing window.
struct Ema {
    value: f64,
    initialized: bool,
    alpha: f64,
}

impl Ema {
    fn new(alpha: f64) -> Self {
        Self {
            value: 0.0,
            initialized: false,
            alpha,
        }
    }

    fn update(&mut self, sample: f64) {
        if self.initialized {
            self.value = self.alpha * sample + (1.0 - self.alpha) * self.value;
        } else {
            self.value = sample;
            self.initialized = true;
        }
    }

    fn get(&self) -> f64 {
        self.value
    }
}

/// Tracks performance metrics for the capture pipeline with EMA smoothing.
pub struct PerfStats {
    last_frame_time: Option<Instant>,
    frame_interval_ema: Ema,

    sampling_send_time: Option<Instant>,
    sampling_time_ema: Ema,

    light_dispatch_ema: Ema,
}

impl PerfStats {
    pub fn new() -> Self {
        let alpha = 0.05;
        Self {
            last_frame_time: None,
            frame_interval_ema: Ema::new(alpha),
            sampling_send_time: None,
            sampling_time_ema: Ema::new(alpha),
            light_dispatch_ema: Ema::new(alpha),
        }
    }

    /// Record arrival of a new frame. Measures interval since the previous frame.
    pub fn record_frame_arrival(&mut self) {
        let now = Instant::now();
        if let Some(prev) = self.last_frame_time {
            let interval_ms = prev.elapsed().as_secs_f64() * 1000.0;
            self.frame_interval_ema.update(interval_ms);
        }
        self.last_frame_time = Some(now);
    }

    /// Mark the start of a sampling request (GPU try_send or CPU loop).
    pub fn mark_sampling_start(&mut self) {
        self.sampling_send_time = Some(Instant::now());
    }

    /// Record completion of sampling. Measures time since `mark_sampling_start`.
    pub fn record_sampling_complete(&mut self) {
        if let Some(start) = self.sampling_send_time.take() {
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            self.sampling_time_ema.update(elapsed_ms);
        }
    }

    /// Record a known sampling duration directly (e.g. measured on the GPU worker thread).
    pub fn record_sampling_time(&mut self, elapsed_ms: f64) {
        self.sampling_time_ema.update(elapsed_ms);
    }

    /// Record a light dispatch round-trip duration.
    pub fn record_light_dispatch(&mut self, elapsed_ms: f64) {
        self.light_dispatch_ema.update(elapsed_ms);
    }

    /// Reset all stats (e.g., when recording stops).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn effective_fps(&self) -> f64 {
        let interval = self.frame_interval_ema.get();
        if interval > 0.0 {
            1000.0 / interval
        } else {
            0.0
        }
    }

    pub fn frame_interval_ms(&self) -> f64 {
        self.frame_interval_ema.get()
    }

    pub fn sampling_time_ms(&self) -> f64 {
        self.sampling_time_ema.get()
    }

    pub fn light_dispatch_ms(&self) -> f64 {
        self.light_dispatch_ema.get()
    }

    pub fn has_frame_data(&self) -> bool {
        self.frame_interval_ema.initialized
    }

    pub fn has_sampling_data(&self) -> bool {
        self.sampling_time_ema.initialized
    }

    pub fn has_light_data(&self) -> bool {
        self.light_dispatch_ema.initialized
    }

    /// Fingerprint for cache invalidation -- rounded metric values.
    pub fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        // DefaultHasher is not stable across Rust versions, but this fingerprint
        // is only used for intra-process cache invalidation so stability is not required.
        let mut h = std::collections::hash_map::DefaultHasher::new();
        // Round to integer to avoid constant cache churn
        (self.effective_fps() as u64).hash(&mut h);
        (self.frame_interval_ms() as u64).hash(&mut h);
        (self.sampling_time_ms() as u64).hash(&mut h);
        (self.light_dispatch_ms() as u64).hash(&mut h);
        h.finish()
    }
}
