import json
from pathlib import Path
import tempfile

import proof


def test_configuration_fixture_preserves_identity_and_disables_both_work_gates():
    payload = {"feature_gates": {"1315865107": {"value": True}, "other": {"value": True}}}
    bootstrap_key = 'svc httpFetch.fetch ["owned",{"url":"/wham/statsig/bootstrap"}]'
    oracle = {
        "shared statsig_evaluations": {
            "accountId": "owned-account",
            "userId": "owned-user",
            "payload": json.dumps(payload),
        },
        bootstrap_key: {"value": {"response": {"__response": {
            "body": json.dumps({"statsigPayload": json.dumps(payload), "access": "unchanged"})
        }}}},
    }
    reply = {"result": {
        "config": {"sandbox_mode": "workspace-write",
                   "sandbox_workspace_write": {"network_access": False}},
        "origins": {},
        "layers": [{"name": {"type": "user"}}],
    }}
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary) / "source"
        local = Path(temporary) / "capture"
        (root / "fixtures/sessions").mkdir(parents=True)
        (root / "fixtures/readback.jsonl").write_text("")
        (local / "tools/route1").mkdir(parents=True)
        oracle_path = local / "tools/route1/oracle.json"
        oracle_path.write_text(json.dumps(oracle))
        reply_path = Path(temporary) / "reply.json"
        reply_path.write_text(json.dumps(reply))
        original_root = proof.ROOT
        try:
            proof.ROOT = root
            proof.configuration_fixture(local, reply_path)
        finally:
            proof.ROOT = original_root
        updated = json.loads(oracle_path.read_text())
        shared = updated["shared statsig_evaluations"]
        assert shared["accountId"] == "owned-account"
        assert shared["userId"] == "owned-user"
        shared_payload = json.loads(shared["payload"])
        assert shared_payload["feature_gates"]["1315865107"]["value"] is False
        assert shared_payload["feature_gates"]["other"]["value"] is True
        body = json.loads(updated[bootstrap_key]["value"]["response"]["__response"]["body"])
        assert body["access"] == "unchanged"
        bootstrap_payload = json.loads(body["statsigPayload"])
        assert bootstrap_payload["feature_gates"]["1315865107"]["value"] is False
        assert bootstrap_payload["feature_gates"]["other"]["value"] is True
        assert (root / "fixtures/readback.jsonl").read_text() == ""
        assert json.loads(reply_path.read_text()) == reply


if __name__ == "__main__":
    test_configuration_fixture_preserves_identity_and_disables_both_work_gates()
    print("PASS: isolated Configuration capability fixture")
