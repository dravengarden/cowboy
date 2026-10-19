"""Account lifecycle checks for the disposable execution-session fixture.

Uses only its authenticated loopback Controller and fresh metadata. No Provider
process, real credential, billing change or production account is involved.
"""
def run(call, require, checks):
    status, before = call("GET", "/api/provider-accounts")
    require(status == 200, "account registry unavailable")
    dataset = before["dataset"]["dataset_id"]
    revision = before["registry"]["revision"]
    def create(operation, label, rev):
        status, result = call("PUT", "/api/provider-accounts", {
            "operation": operation, "dataset": dataset, "expected_revision": rev,
            "change": {"type": "create", "provider_id": "claude-code", "label": label}})
        require(status == 200 and result["status"] == "committed", "account fixture create failed")
        return result
    first = create("account-create-1", "First", revision)
    second = create("account-create-2", "Second", first["revision"])
    account = first["account_id"]
    path = f"/api/provider-accounts/{account}/revocation"
    body = {"operation": "account-revoke", "dataset": dataset, "expected_revision": second["revision"]}
    require(call("POST", path, body, anonymous=True)[0] in (401, 403), "anonymous revocation admitted")
    require(call("POST", path, {**body, "dataset": "wrong"})[0] == 409, "wrong dataset admitted")
    require(call("POST", path, {**body, "expected_revision": 0})[0] == 409, "stale revocation admitted")
    require(call("GET", path)[1]["status"] == "not_started", "rejected requests created an intent")
    require(call("PUT", "/api/provider-accounts", {
        **body, "change": {"type": "begin_revocation", "id": account}})[0] == 422,
        "metadata writer accepted an internal lifecycle variant")
    status, committed = call("POST", path, body)
    require(status == 200 and committed["status"] == "revoking", "revocation intent not committed")
    require(call("POST", path, body)[1] == committed, "same operation did not replay the original receipt")
    require(call("POST", path, {**body, "operation": "duplicate", "expected_revision": committed["revision"]})[0] == 409,
        "a second operation replaced the original revocation plan")
    _, registry = call("GET", "/api/provider-accounts")
    accounts = {item["id"]: item for item in registry["registry"]["accounts"]}
    require(accounts[account]["revoking"] and not accounts[account]["enabled"], "revocation did not fence admission")
    require(accounts[second["account_id"]]["enabled"] and not accounts[second["account_id"]]["revoking"], "another account changed")
    checks.append("public_revocation_is_scoped_dataset_bound_and_replays_one_immutable_plan")
    require(call("POST", path + "/reconcile", {"dataset": "wrong"})[0] == 409, "wrong dataset resumed cleanup")
    status, result = call("POST", path + "/reconcile", {"dataset": dataset})
    require(status == 200 and result["status"] == "completed", "empty native plan cleanup did not complete")
    status, observed = call("GET", path)
    require(status == 200 and observed["status"] == "completed" and observed["completion"]["account_completed"]
            and not observed["completion"]["missing"], "completed cleanup not recoverable by account")
    require(call("POST", path + "/reconcile", {"dataset": dataset})[1]["status"] == "completed", "completed retry changed outcome")
    require(call("PUT", "/api/provider-accounts", {"operation": "reenable-revoked", "dataset": dataset,
        "expected_revision": committed["revision"], "change": {"type": "set_enabled", "id": account, "enabled": True}})[0] == 422,
        "metadata enablement revived revoked access")
    checks.append("public_cleanup_completion_is_observable_and_cannot_be_cleared_by_enablement")
