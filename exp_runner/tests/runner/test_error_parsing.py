import pandas as pd
import pytest

from exp_runner.runner.plotting.util import _parse_error_columns


class TestErrorParsing:
    def test_standard_early_return(self):
        # Old: /EarlyReturn:frontend:Login
        # New: /EarlyReturn?src=frontend::Login
        df = pd.DataFrame({"error": ["/EarlyReturn?src=frontend::Login"]})
        df = _parse_error_columns(df)

        assert df.iloc[0]["error_type"] == "EarlyReturn"
        assert df.iloc[0]["er_service"] == "frontend"
        assert df.iloc[0]["er_method"] == "Login"
        # er_last_child should be NaN/None
        assert (
            pd.isna(df.iloc[0]["er_last_child"]) or df.iloc[0]["er_last_child"] is None
        )

    def test_early_return_with_last_child(self):
        # Old: /EarlyReturn:frontend:Login|/user.Service/GetUser
        # New: /EarlyReturn?src=frontend::Login?last_rpc=user.Service::GetUser
        df = pd.DataFrame(
            {
                "error": [
                    "/EarlyReturn?src=frontend::Login?last_rpc=user.Service::GetUser"
                ]
            }
        )
        df = _parse_error_columns(df)

        assert df.iloc[0]["error_type"] == "EarlyReturn"
        assert df.iloc[0]["er_service"] == "frontend"
        assert df.iloc[0]["er_method"] == "Login"
        assert df.iloc[0]["er_last_child"] == "user.Service::GetUser"

    def test_generic_error(self):
        df = pd.DataFrame({"error": ["/ClientTimeout", "RPC Error", ""]})
        df = _parse_error_columns(df)

        assert (df["error_type"] == "Generic").all()
        assert (df["er_service"].isna()).all()

    def test_mixed_errors(self):
        data = [
            "/EarlyReturn?src=A::Op1",
            "/EarlyReturn?src=B::Op2?last_rpc=Child",
            "/ClientTimeout",
            "SomeOtherError",
        ]
        df = pd.DataFrame({"error": data})
        df = _parse_error_columns(df)

        assert df.iloc[0]["error_type"] == "EarlyReturn"
        assert df.iloc[0]["er_service"] == "A"

        assert df.iloc[1]["error_type"] == "EarlyReturn"
        assert df.iloc[1]["er_last_child"] == "Child"

        assert df.iloc[2]["error_type"] == "Generic"

    def test_no_error_column(self):
        df = pd.DataFrame({"other": [1, 2, 3]})
        df = _parse_error_columns(df)
        assert "error_type" in df.columns  # It creates default columns
        assert df.iloc[0]["error_type"] == "Generic"

    def test_empty_dataframe(self):
        df = pd.DataFrame({"error": []})
        df = _parse_error_columns(df)
        assert "error_type" in df.columns
        assert df.empty
