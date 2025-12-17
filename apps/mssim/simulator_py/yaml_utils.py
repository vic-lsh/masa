from __future__ import annotations

import json
from typing import Any, List


def dump_yaml(data: Any) -> str:
    lines: List[str] = []
    _serialize(data, lines, indent=0, is_sequence_item=False)
    return "\n".join(lines) + "\n"


def _serialize(value: Any, lines: List[str], indent: int, is_sequence_item: bool) -> None:
    prefix = "  " * indent
    if isinstance(value, dict):
        if not value:
            lines.append(f"{prefix}{{}}")
            return
        for key, item in value.items():
            key_str = str(key)
            if isinstance(item, (dict, list)):
                lines.append(f"{prefix}{key_str}:")
                _serialize(item, lines, indent + 1, is_sequence_item=False)
            else:
                scalar = _format_scalar(item)
                lines.append(f"{prefix}{key_str}: {scalar}")
    elif isinstance(value, list):
        if not value:
            lines.append(f"{prefix}[]")
            return
        for item in value:
            if isinstance(item, (dict, list)):
                lines.append(f"{prefix}-")
                _serialize(item, lines, indent + 1, is_sequence_item=True)
            else:
                scalar = _format_scalar(item)
                lines.append(f"{prefix}- {scalar}")
    else:
        scalar = _format_scalar(value)
        if is_sequence_item:
            lines.append(f"{prefix}{scalar}")
        else:
            lines.append(f"{prefix}{scalar}")


def _format_scalar(value: Any) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if value is None:
        return "null"
    if isinstance(value, (int, float)):
        return str(value)
    return json.dumps(str(value))

