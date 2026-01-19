// Strong types for synthetic application

use std::fmt;
use std::str::FromStr;

/// Strongly typed service identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServiceId(String);

impl ServiceId {
    pub fn new(id: impl Into<String>) -> Result<Self, SyntheticError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(SyntheticError::InvalidServiceId(
                "Service ID cannot be empty".to_string(),
            ));
        }
        Ok(ServiceId(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for ServiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ServiceId {
    type Err = SyntheticError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s.to_string())
    }
}

/// Strongly typed method name
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MethodName(String);

impl MethodName {
    pub fn new(name: impl Into<String>) -> Result<Self, SyntheticError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(SyntheticError::InvalidMethodName(
                "Method name cannot be empty".to_string(),
            ));
        }
        Ok(MethodName(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for MethodName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for MethodName {
    type Err = SyntheticError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s.to_string())
    }
}

/// Strongly typed probability value (0.0 to 1.0)
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Probability(f64);

impl Probability {
    pub fn new(p: f64) -> Result<Self, SyntheticError> {
        if p < 0.0 || p > 1.0 {
            return Err(SyntheticError::InvalidProbability(format!(
                "Probability must be between 0.0 and 1.0, got {}",
                p
            )));
        }
        Ok(Probability(p))
    }

    pub fn value(self) -> f64 {
        self.0
    }

    pub fn should_make_call(self) -> bool {
        let mut rng = rand::thread_rng();
        rng.gen::<f64>() < self.0
    }

    pub const fn always() -> Self {
        Probability(1.0)
    }

    pub const fn never() -> Self {
        Probability(0.0)
    }
}

impl fmt::Display for Probability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.3}", self.0)
    }
}

/// Strongly typed duration in microseconds
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DurationMicros(u64);

impl DurationMicros {
    pub fn new(us: u64) -> Result<Self, SyntheticError> {
        if us == 0 {
            return Err(SyntheticError::InvalidDuration(
                "Duration must be greater than 0".to_string(),
            ));
        }
        Ok(DurationMicros(us))
    }

    pub fn new_allow_zero(us: u64) -> Self {
        DurationMicros(us)
    }

    pub fn value(self) -> u64 {
        self.0
    }

    pub fn saturating_sub(self, other: Self) -> Self {
        DurationMicros(self.0.saturating_sub(other.0))
    }

    pub fn ratio(self, other: Self) -> f64 {
        if other.0 == 0 {
            0.0
        } else {
            self.0 as f64 / other.0 as f64
        }
    }
}

impl fmt::Display for DurationMicros {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}μs", self.0)
    }
}

impl From<DurationMicros> for std::time::Duration {
    fn from(duration: DurationMicros) -> Self {
        std::time::Duration::from_micros(duration.0)
    }
}

use crate::error::SyntheticError;
use rand::Rng;
