import pytest

from exp.runner.plotting.util import get_policy_display_name


def test_get_policy_display_name_known_policies():
    assert get_policy_display_name("fifo") == "FIFO (no-drop)"
    assert get_policy_display_name("fifo,early") == "FIFO"
    assert get_policy_display_name("prio_global") == "Masa (global ddl) (no-drop)"
    assert get_policy_display_name("prio_global,early") == "Masa (global ddl)"
    assert get_policy_display_name("prio_local") == "Masa (local ddl) (no-drop)"
    assert get_policy_display_name("prio_local,early") == "Masa (local ddl)"
    assert get_policy_display_name("prio_oldest") == "Tailclipper (no-drop)"
    assert get_policy_display_name("prio_oldest,early") == "Tailclipper"


def test_get_policy_display_name_unknown_policies():
    assert get_policy_display_name("custom") == "custom (no-drop)"
    assert get_policy_display_name("custom,early") == "custom"
