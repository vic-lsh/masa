"""Tests for naming module."""

from exp_runner.runner.naming import generate_project_name, truncate_experiment_slug


def test_generate_project_name_basic():
    """Test basic project name generation."""
    name = generate_project_name("hotel", "baseline", 0, "fifo")

    # Should have format: app_prefix-slug-digest
    parts = name.split("-")
    assert len(parts) >= 4  # hot-baseline-0-fifo-<digest>
    assert parts[0] == "hot"  # hotel prefix
    assert parts[1] == "baseline"
    assert parts[2] == "0"
    assert parts[3] == "fifo"
    assert len(parts[4]) == 8  # digest is 8 chars


def test_generate_project_name_with_rps():
    """Test project name generation with RPS."""
    name = generate_project_name("mssim", "S_14677443", 0, "fifo", 100.0)

    parts = name.split("-")
    assert parts[0] == "mss"  # mssim prefix
    # Should include RPS
    assert "100" in name


def test_generate_project_name_deterministic():
    """Test that same inputs produce same project name."""
    name1 = generate_project_name("hotel", "baseline", 0, "fifo")
    name2 = generate_project_name("hotel", "baseline", 0, "fifo")
    assert name1 == name2


def test_generate_project_name_unique_for_different_inputs():
    """Test that different inputs produce different project names."""
    name1 = generate_project_name("hotel", "baseline", 0, "fifo")
    name2 = generate_project_name("hotel", "baseline", 1, "fifo")
    name3 = generate_project_name("hotel", "baseline", 0, "prio_global")

    # All should be different
    assert name1 != name2
    assert name1 != name3
    assert name2 != name3


def test_generate_project_name_lowercase():
    """Test that project names are always lowercase."""
    name = generate_project_name("HOTEL", "BASELINE", 0, "FIFO")
    assert name == name.lower()


def test_generate_project_name_synthetic():
    """Test project name for synthetic app."""
    name = generate_project_name("synthetic", "fanout-test", 2, "prio_global")

    assert name.startswith("syn-")
    assert "fanout-test" in name
    assert "2" in name
    assert "prio" in name or "global" in name  # policy included


def test_truncate_experiment_slug_no_truncation():
    """Test truncate_experiment_slug when no truncation needed."""
    slug = truncate_experiment_slug("short-name", 20)
    assert slug == "short-name"


def test_truncate_experiment_slug_with_truncation():
    """Test truncate_experiment_slug when truncation needed."""
    long_name = "very-long-experiment-name-here"
    slug = truncate_experiment_slug(long_name, 15)

    assert len(slug) == 15
    assert slug.endswith("...")
    assert slug.startswith("very-long-e")  # First part preserved
