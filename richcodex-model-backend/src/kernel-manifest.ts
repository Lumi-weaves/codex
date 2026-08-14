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
  selectionDigest: "sha256:5e7c03c78ba23105858523d923f000bfcb0ba6f352395fd5f72cdf823c49c97a",
  compositionVersion: 4,
} as const);

export type RichCodexBackendKernel = typeof RICHCODEX_BACKEND_KERNEL;
