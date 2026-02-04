use app_utils::timing::time_now;
use rand::thread_rng;
use rand_distr::{Distribution, Exp, LogNormal, Normal, WeightedIndex};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

fn lognormal_from_mean_std(mean: f64, std: f64) -> Result<LogNormal<f64>, String> {
    if mean <= 0.0 {
        return Err("LogNormal mean must be positive".to_string());
    }
    if std <= 0.0 {
        return Err("LogNormal std must be positive".to_string());
    }
    // Convert mean/std of X into mu/sigma of ln(X).
    let variance = std * std;
    let mean_sq = mean * mean;
    let sigma_sq = (1.0 + (variance / mean_sq)).ln();
    let sigma = sigma_sq.sqrt();
    let mu = mean.ln() - 0.5 * sigma_sq;
    LogNormal::new(mu, sigma).map_err(|e| e.to_string())
}

fn lognormal_from_p50_p95(p50: f64, p95: f64) -> Result<(f64, f64, LogNormal<f64>), String> {
    if p50 <= 0.0 {
        return Err("LogNormal p50 must be positive".to_string());
    }
    if p95 <= 0.0 {
        return Err("LogNormal p95 must be positive".to_string());
    }
    if p95 <= p50 {
        return Err("LogNormal p95 must be greater than p50".to_string());
    }

    // For lognormal: median = exp(mu) and p95 = exp(mu + sigma * z95).
    let z95 = 1.644_853_626_951_472_2_f64;
    let mu = p50.ln();
    let sigma = (p95.ln() - p50.ln()) / z95;

    // Convert mu/sigma to mean/std of X for storage.
    let sigma_sq = sigma * sigma;
    let mean = (mu + 0.5 * sigma_sq).exp();
    let variance = (sigma_sq.exp() - 1.0) * (2.0 * mu + sigma_sq).exp();
    let std = variance.sqrt();

    let dist = LogNormal::new(mu, sigma).map_err(|e| e.to_string())?;
    Ok((mean, std, dist))
}

#[derive(Debug)]
pub enum LatencyDistribution {
    Normal {
        mean: f64,
        std: f64,
        dist: Normal<f64>,
    },
    Exponential {
        lambda: f64,
        mean: Option<f64>,
        dist: Exp<f64>,
    },
    LogNormal {
        mean: f64,
        std: f64,
        dist: LogNormal<f64>,
    },
    Discrete {
        weights: Vec<f64>,
        values: Vec<u64>,
        dist: WeightedIndex<f64>,
    },
    Periodic {
        slow_latency: u64,
        fast_latency: u64,
        slow_duration_ms: u16,
    },
}

impl LatencyDistribution {
    // returns latency in us
    pub fn sample(&self) -> u64 {
        match self {
            LatencyDistribution::Normal { dist, .. } => {
                let mut l = dist.sample(&mut thread_rng()).round();

                // make sure latency is non-negative
                if l < 0.0 {
                    l = 0.0;
                }

                l as u64
            }
            LatencyDistribution::Exponential { dist, .. } => {
                dist.sample(&mut thread_rng()).round() as u64
            }
            LatencyDistribution::LogNormal { dist, .. } => {
                dist.sample(&mut thread_rng()).round() as u64
            }
            LatencyDistribution::Discrete { dist, values, .. } => {
                values[dist.sample(&mut thread_rng())]
            }
            LatencyDistribution::Periodic {
                slow_latency,
                fast_latency,
                slow_duration_ms,
            } => {
                let now_ms = (time_now() / 1000) % 1000;

                if now_ms < *slow_duration_ms as u64 {
                    *slow_latency
                } else {
                    *fast_latency
                }
            }
        }
    }
}

impl Clone for LatencyDistribution {
    fn clone(&self) -> Self {
        match self {
            LatencyDistribution::Normal { mean, std, .. } => LatencyDistribution::Normal {
                mean: *mean,
                std: *std,
                dist: Normal::new(*mean, *std).unwrap(),
            },
            LatencyDistribution::Exponential { lambda, mean, .. } => {
                LatencyDistribution::Exponential {
                    lambda: *lambda,
                    mean: *mean,
                    dist: Exp::new(*lambda).unwrap(),
                }
            }
            LatencyDistribution::LogNormal { mean, std, .. } => LatencyDistribution::LogNormal {
                mean: *mean,
                std: *std,
                dist: lognormal_from_mean_std(*mean, *std).unwrap(),
            },

            LatencyDistribution::Discrete {
                weights, values, ..
            } => LatencyDistribution::Discrete {
                weights: weights.clone(),
                values: values.clone(),
                dist: WeightedIndex::new(weights.clone()).unwrap(),
            },
            LatencyDistribution::Periodic {
                slow_latency,
                fast_latency,
                slow_duration_ms,
            } => LatencyDistribution::Periodic {
                slow_latency: *slow_latency,
                fast_latency: *fast_latency,
                slow_duration_ms: *slow_duration_ms,
            },
        }
    }
}

impl Serialize for LatencyDistribution {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        enum LatencyDistributionSer {
            Normal {
                mean: f64,
                std: f64,
            },
            Exponential {
                lambda: f64,
                #[serde(skip_serializing_if = "Option::is_none")]
                mean: Option<f64>,
            },
            LogNormal {
                mean: f64,
                std: f64,
            },
            Discrete {
                weights: Vec<f64>,
                values: Vec<u64>,
            },
            Periodic {
                slow_latency: u64,
                fast_latency: u64,
                slow_duration_ms: u16,
            },
        }

        let ser = match self {
            LatencyDistribution::Normal { mean, std, .. } => LatencyDistributionSer::Normal {
                mean: *mean,
                std: *std,
            },
            LatencyDistribution::Exponential { lambda, mean, .. } => {
                LatencyDistributionSer::Exponential {
                    lambda: *lambda,
                    mean: *mean,
                }
            }
            LatencyDistribution::LogNormal { mean, std, .. } => LatencyDistributionSer::LogNormal {
                mean: *mean,
                std: *std,
            },
            LatencyDistribution::Discrete {
                weights, values, ..
            } => LatencyDistributionSer::Discrete {
                weights: weights.clone(),
                values: values.clone(),
            },
            LatencyDistribution::Periodic {
                slow_latency,
                fast_latency,
                slow_duration_ms,
            } => LatencyDistributionSer::Periodic {
                slow_latency: *slow_latency,
                fast_latency: *fast_latency,
                slow_duration_ms: *slow_duration_ms,
            },
        };
        ser.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LatencyDistribution {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        enum LatencyDistributionDe {
            Normal {
                mean: f64,
                std: f64,
            },
            Exponential {
                #[serde(default)]
                lambda: Option<f64>,
                #[serde(default)]
                mean: Option<f64>,
            },
            LogNormal {
                #[serde(default)]
                mean: Option<f64>,
                #[serde(default)]
                std: Option<f64>,
                #[serde(default)]
                p50: Option<f64>,
                #[serde(default)]
                p95: Option<f64>,
            },
            Discrete {
                weights: Vec<f64>,
                values: Vec<u64>,
            },
            Periodic {
                slow_latency: u64,
                fast_latency: u64,
                slow_duration_ms: u16,
            },
        }

        let de = LatencyDistributionDe::deserialize(deserializer)?;
        Ok(match de {
            LatencyDistributionDe::Normal { mean, std } => LatencyDistribution::Normal {
                mean,
                std,
                dist: Normal::new(mean, std).map_err(serde::de::Error::custom)?,
            },
            LatencyDistributionDe::Exponential { lambda, mean } => {
                let (lambda_val, mean_val) = match (lambda, mean) {
                    (Some(_l), Some(m)) => {
                        // If both provided, prefer mean
                        let lambda_from_mean = 1.0 / m;
                        (lambda_from_mean, Some(m))
                    }
                    (Some(l), None) => (l, None),
                    (None, Some(m)) => {
                        if m <= 0.0 {
                            return Err(serde::de::Error::custom(
                                "Exponential mean must be positive",
                            ));
                        }
                        (1.0 / m, Some(m))
                    }
                    (None, None) => {
                        return Err(serde::de::Error::custom(
                            "Exponential distribution requires either 'lambda' or 'mean' parameter",
                        ));
                    }
                };
                if lambda_val <= 0.0 {
                    return Err(serde::de::Error::custom(
                        "Exponential lambda must be positive",
                    ));
                }
                LatencyDistribution::Exponential {
                    lambda: lambda_val,
                    mean: mean_val,
                    dist: Exp::new(lambda_val).map_err(serde::de::Error::custom)?,
                }
            }
            LatencyDistributionDe::LogNormal {
                mean,
                std,
                p50,
                p95,
            } => {
                let (mean_val, std_val, dist) = match (p50, p95, mean, std) {
                    (Some(p50_val), Some(p95_val), _, _) => {
                        lognormal_from_p50_p95(p50_val, p95_val)
                            .map_err(serde::de::Error::custom)?
                    }
                    (None, None, Some(mean_val), Some(std_val)) => {
                        let dist = lognormal_from_mean_std(mean_val, std_val)
                            .map_err(serde::de::Error::custom)?;
                        (mean_val, std_val, dist)
                    }
                    _ => {
                        return Err(serde::de::Error::custom(
                            "LogNormal requires either 'p50' and 'p95' or 'mean' and 'std'",
                        ));
                    }
                };
                LatencyDistribution::LogNormal {
                    mean: mean_val,
                    std: std_val,
                    dist,
                }
            }
            LatencyDistributionDe::Discrete { weights, values } => {
                assert_eq!(weights.len(), values.len());
                LatencyDistribution::Discrete {
                    weights: weights.clone(),
                    values: values.clone(),
                    dist: WeightedIndex::new(weights).map_err(serde::de::Error::custom)?,
                }
            }
            LatencyDistributionDe::Periodic {
                slow_latency,
                fast_latency,
                slow_duration_ms,
            } => LatencyDistribution::Periodic {
                slow_latency,
                fast_latency,
                slow_duration_ms,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_lognormal_from_mean_std_direct() {
        // Test valid input
        let res = lognormal_from_mean_std(100.0, 10.0);
        assert!(res.is_ok());

        // Test invalid inputs
        assert!(lognormal_from_mean_std(-10.0, 10.0).is_err());
        assert!(lognormal_from_mean_std(100.0, -10.0).is_err());
    }

    #[test]
    fn test_lognormal_from_p50_p95_direct() {
        // Test valid input
        let res = lognormal_from_p50_p95(100.0, 200.0);
        assert!(res.is_ok());
        let (mean, std, _dist) = res.unwrap();
        // Check that mean > median
        assert!(mean > 100.0);
        assert!(std > 0.0);

        // Test invalid inputs
        assert!(lognormal_from_p50_p95(-10.0, 200.0).is_err());
        assert!(lognormal_from_p50_p95(100.0, -200.0).is_err());
        assert!(lognormal_from_p50_p95(100.0, 90.0).is_err()); // p95 < p50
    }

    #[test]
    fn test_serde_lognormal_config() {
        // Test configuring via mean/std
        let config_mean_std = json!({
            "LogNormal": {
                "mean": 100.0,
                "std": 10.0
            }
        });
        let dist: LatencyDistribution =
            serde_json::from_value(config_mean_std).expect("parse lognormal mean/std");
        if let LatencyDistribution::LogNormal { mean, std, .. } = dist {
            assert_eq!(mean, 100.0);
            assert_eq!(std, 10.0);
        } else {
            panic!("Expected LogNormal");
        }

        // Test configuring via p50/p95
        let config_p50_p95 = json!({
            "LogNormal": {
                "p50": 100.0,
                "p95": 200.0
            }
        });
        let dist: LatencyDistribution =
            serde_json::from_value(config_p50_p95).expect("parse lognormal p50/p95");
        if let LatencyDistribution::LogNormal { mean, .. } = dist {
            assert!(mean > 100.0);
        } else {
            panic!("Expected LogNormal");
        }
    }

    #[test]
    fn test_lognormal_sampling() {
        // Create a distribution with known parameters
        let dist = LatencyDistribution::LogNormal {
            mean: 1000.0,
            std: 100.0,
            dist: lognormal_from_mean_std(1000.0, 100.0).unwrap(),
        };

        // Sample many times
        let mut sum = 0.0;
        let n = 10000;
        for _ in 0..n {
            sum += dist.sample() as f64;
        }
        let computed_mean = sum / n as f64;

        // The sample mean should be reasonably close to the configured mean
        assert!(
            (computed_mean - 1000.0).abs() < 50.0,
            "Sample mean {} should be close to 1000.0",
            computed_mean
        );
    }
}
