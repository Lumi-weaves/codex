/**
 * Build-owned provenance for the bundled backend kernel.
 *
 * `contentDigest` is the verified digest of the selected upstream source
 * snapshot: `git archive --format=tar cbbfdd8773e68a5dc2391ddeb32f33a225373c1a | sha256sum`.
 * It is intentionally recorded as a constant rather than derived from ambient
 * git, npm, or an installed OpenCodex runtime.
 */
export const RICHCODEX_BACKEND_KERNEL = Object.freeze({
  sourceRepository: "https://github.com/lidge-jun/opencodex",
  sourceCommit: "cbbfdd8773e68a5dc2391ddeb32f33a225373c1a",
  contentDigest: "sha256:65672062788957661574aafd6d32d571d0a33afb0575f6a12e19801d72874b78",
  selectionDigest: "sha256:fd809eafabdcd42b72ebce5dc9ff9faf93e7a279fe0f12acc794dbc124d23808",
  compositionVersion: 2,
} as const);

export type RichCodexBackendKernel = typeof RICHCODEX_BACKEND_KERNEL;
