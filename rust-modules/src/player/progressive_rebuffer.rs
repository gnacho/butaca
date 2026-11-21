//! Progressive Original starvation detection and recovery contracts.
//!
//! The detector follows plx-native `9f5ff482`: a Page CUSUM of wall time minus native position,
//! with a drift allowance and a queue-empty gate. The values are upstream's device-measured starting
//! point, not a Butaca television measurement. Device validation remains a separate acceptance gate.

const MIN_POSITION_NS: i64 = 2_000_000_000;
const STALE_GAP_MS: i32 = 1_000;
const DRIFT_US_PER_MS: i64 = 50;
const FIRE_THRESHOLD_US: i64 = 100_000;
const QUEUE_GATE_BYTES: usize = 1 << 20;

#[derive(Clone, Copy, Debug)]
pub(crate) struct QueueStatus {
    pub bytes: i64,
    pub cap: i64,
    pub eof: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResumeReason {
    Full,
    Eof,
}

impl ResumeReason {
    pub(crate) fn word(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Eof => "eof",
        }
    }
}

pub(crate) fn resume_reason(video: QueueStatus, audio: QueueStatus) -> Option<ResumeReason> {
    if video.eof || audio.eof {
        return Some(ResumeReason::Eof);
    }
    let full = |q: QueueStatus| q.cap > 0 && q.bytes >= q.cap;
    (full(video) || full(audio)).then_some(ResumeReason::Full)
}

pub(crate) fn reserve_ready(video: QueueStatus, audio: QueueStatus) -> bool {
    resume_reason(video, audio).is_some()
}

pub(crate) fn route_eligible(
    quality: crate::route::Quality,
    transcoding: bool,
    preview: bool,
    auto_original_watched: bool,
) -> bool {
    quality == crate::route::Quality::Original && !transcoding && !preview && !auto_original_watched
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ClockDebt {
    #[default]
    Ineligible,
    Watching {
        wall_ms: u32,
        pos_ns: i64,
        s_us: i64,
    },
    Fired {
        s_us: i64,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DebtSample {
    pub wall_ms: u32,
    pub pos_ns: i64,
    pub video_queue_bytes: usize,
    pub eligible: bool,
}

impl ClockDebt {
    pub(crate) fn step(self, sample: DebtSample) -> ClockDebt {
        if !sample.eligible || sample.pos_ns <= MIN_POSITION_NS {
            return Self::Ineligible;
        }
        let start = ClockDebt::Watching {
            wall_ms: sample.wall_ms,
            pos_ns: sample.pos_ns,
            s_us: 0,
        };
        if sample.video_queue_bytes > QUEUE_GATE_BYTES {
            return start;
        }
        match self {
            Self::Ineligible => start,
            Self::Fired { .. } => self,
            Self::Watching {
                wall_ms,
                pos_ns,
                s_us,
            } => {
                let dt_ms = sample.wall_ms.wrapping_sub(wall_ms) as i32;
                if dt_ms <= 0 || dt_ms > STALE_GAP_MS {
                    return start;
                }
                let dt_ms = i64::from(dt_ms);
                let debt_us = dt_ms * 1_000 - (sample.pos_ns - pos_ns) / 1_000;
                let s_us = (s_us + debt_us - DRIFT_US_PER_MS * dt_ms).max(0);
                if s_us >= FIRE_THRESHOLD_US {
                    Self::Fired { s_us }
                } else {
                    Self::Watching {
                        wall_ms: sample.wall_ms,
                        pos_ns: sample.pos_ns,
                        s_us,
                    }
                }
            }
        }
    }

    pub(crate) fn s_us(self) -> i64 {
        match self {
            Self::Ineligible => 0,
            Self::Watching { s_us, .. } | Self::Fired { s_us } => s_us,
        }
    }

    pub(crate) fn fired(self) -> bool {
        matches!(self, Self::Fired { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START_NS: i64 = 10_000_000_000;

    fn sample(wall_ms: u32, pos_ns: i64) -> DebtSample {
        DebtSample {
            wall_ms,
            pos_ns,
            video_queue_bytes: 0,
            eligible: true,
        }
    }

    #[test]
    fn a_clock_inside_the_drift_allowance_never_fires() {
        let mut debt = ClockDebt::default().step(sample(1_000, START_NS));
        for i in 1..=100i64 {
            debt = debt.step(sample(1_000 + i as u32 * 200, START_NS + i * 192_000_000));
        }
        assert!(!debt.fired());
        assert_eq!(debt.s_us(), 0);
    }

    #[test]
    fn a_refilled_queue_resets_a_fired_debt() {
        let debt = ClockDebt::default().step(sample(1_000, START_NS));
        let debt = debt.step(sample(1_200, START_NS + 100_000_000));
        let debt = debt.step(sample(1_400, START_NS + 200_000_000));
        assert!(debt.fired());
        let mut refilled = sample(1_600, START_NS + 300_000_000);
        refilled.video_queue_bytes = 2 << 20;
        let debt = debt.step(refilled);
        assert!(!debt.fired());
        assert_eq!(debt.s_us(), 0);
    }

    #[test]
    fn release_reason_names_a_full_reserve_or_producer_eof() {
        let empty = QueueStatus {
            bytes: 0,
            cap: 100,
            eof: false,
        };
        assert_eq!(
            resume_reason(
                QueueStatus {
                    bytes: 100,
                    ..empty
                },
                empty
            ),
            Some(ResumeReason::Full)
        );
        assert_eq!(
            resume_reason(empty, QueueStatus { eof: true, ..empty }),
            Some(ResumeReason::Eof)
        );
        assert_eq!(resume_reason(empty, empty), None);
        assert_eq!(ResumeReason::Full.word(), "full");
        assert_eq!(ResumeReason::Eof.word(), "eof");
    }

    #[test]
    fn a_full_lane_or_producer_eof_releases_the_hold() {
        let empty = QueueStatus {
            bytes: 0,
            cap: 100,
            eof: false,
        };
        let absent = QueueStatus {
            bytes: 0,
            cap: 0,
            eof: false,
        };
        assert!(!reserve_ready(empty, empty));
        assert!(!reserve_ready(absent, absent));
        assert!(reserve_ready(
            QueueStatus {
                bytes: 100,
                ..empty
            },
            empty
        ));
        assert!(reserve_ready(
            empty,
            QueueStatus {
                bytes: 120,
                ..empty
            }
        ));
        assert!(reserve_ready(QueueStatus { eof: true, ..empty }, empty));
        assert!(reserve_ready(empty, QueueStatus { eof: true, ..empty }));
    }

    #[test]
    fn only_unsupervised_progressive_original_is_eligible() {
        use crate::route::Quality;
        assert!(route_eligible(Quality::Original, false, false, false));
        assert!(!route_eligible(Quality::Auto, false, false, true));
        assert!(!route_eligible(Quality::Original, true, false, false));
        assert!(!route_eligible(Quality::Original, false, true, false));
        assert!(!route_eligible(Quality::Original, false, false, true));
        assert!(!route_eligible(Quality::P1080, true, false, false));
    }

    #[test]
    fn an_early_position_never_accumulates_debt() {
        let debt = ClockDebt::default().step(sample(1_000, 100_000_000));
        let debt = debt.step(sample(1_200, 200_000_000));
        let debt = debt.step(sample(1_400, 300_000_000));
        assert_eq!(debt, ClockDebt::Ineligible);
    }

    #[test]
    fn an_ineligible_sample_clears_a_fired_debt() {
        let debt = ClockDebt::default().step(sample(1_000, START_NS));
        let debt = debt.step(sample(1_200, START_NS + 100_000_000));
        let debt = debt.step(sample(1_400, START_NS + 200_000_000));
        assert!(debt.fired());
        let mut paused = sample(1_600, START_NS + 200_000_000);
        paused.eligible = false;
        assert_eq!(debt.step(paused), ClockDebt::Ineligible);
    }

    #[test]
    fn a_slow_clock_fires_after_accumulating_more_than_the_drift_allowance() {
        let debt = ClockDebt::default().step(sample(1_000, START_NS));
        let debt = debt.step(sample(1_200, START_NS + 100_000_000));
        assert_eq!(debt.s_us(), 90_000);
        assert!(!debt.fired());
        let debt = debt.step(sample(1_400, START_NS + 200_000_000));
        assert_eq!(debt.s_us(), 180_000);
        assert!(debt.fired());
    }
}
