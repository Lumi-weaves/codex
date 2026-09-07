import { expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createModelPlaneStore } from "../src/model-plane";

test("logout erases secrets, preserves routes across restart and permits reauthentication", () => {
  const root = mkdtempSync(join(tmpdir(), "enclave-logout-"));
  try {
    const now = 1_900_000_000_000;
    let store = createModelPlaneStore(root, { now: () => now });
    const credential = { kind: "oauth" as const, accessToken: "logout-canary-access",
      refreshToken: "logout-canary-refresh", chatgptAccountId: "subject", expiresAt: now + 3600000 };
    const account = store.addOAuthAccount(credential, "Personal");
    for (const model of ["sol", "astra"]) store.createModelRoute({
      expectedRevision: store.snapshot().desiredStateRevision, modelTag: model,
      displayName: model, semanticModel: model, providerId: "openai", accountId: account.id,
      upstreamModelId: model,
    });
    const routes = store.snapshot().modelRoutes.map(r => ({ ...r, targets: r.targets.map(t => ({ ...t, status: "reauthenticationRequired" as const })) }));
    const revision = store.snapshot().desiredStateRevision;
    expect(() => store.logoutAccount(account.id, revision - 1)).toThrow("revision_conflict");
    expect(store.logoutAccount(account.id, revision).status).toBe("reauthenticationRequired");
    const persisted = readFileSync(join(root, "model-plane.json"), "utf8");
    expect(persisted.includes(credential.accessToken)).toBe(false);
    expect(persisted.includes(credential.refreshToken)).toBe(false);
    expect(store.replaceOAuthCredential(account.id, credential.refreshToken, credential)).toBe(false);
    store.markAccountStatus(account.id, "ready");
    store = createModelPlaneStore(root, { now: () => now });
    expect(store.snapshot().modelRoutes).toEqual(routes);
    expect(store.snapshot().accounts[0]?.status).toBe("reauthenticationRequired");
    expect(store.resolveExecutionCandidates("sol")).toEqual([]);
    expect(() => store.readOAuthAccessToken(account.id)).toThrow();
    expect(store.reauthenticateOAuthAccount(account.id, { ...credential, chatgptAccountId: "new-subject" }).id).toBe(account.id);
    expect(store.resolveExecutionCandidates("astra")).toHaveLength(1);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
