pub mod alibaba;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    DockerCompose,
    Kubernetes,
}

impl Backend {
    pub fn variants() -> &'static [&'static str] {
        &["docker-compose", "k8s"]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Backend::DockerCompose => "docker-compose",
            Backend::Kubernetes => "k8s",
        }
    }
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Backend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "docker" | "docker-compose" => Ok(Backend::DockerCompose),
            "k8s" | "kubernetes" => Ok(Backend::Kubernetes),
            other => Err(format!("Unsupported orchestrator backend: {}", other)),
        }
    }
}
