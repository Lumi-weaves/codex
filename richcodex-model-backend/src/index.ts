import {
  createHeadlessBackend,
  HeadlessBackendConfigurationError,
  parseHeadlessBackendArgs,
  type HeadlessBackendInput,
} from "./backend";

export * from "./backend";
export * from "./kernel-manifest";

interface ProcessIo {
  readonly stdin: HeadlessBackendInput;
  readonly stdout: { write(chunk: string): unknown };
  readonly stderr: { write(chunk: string): unknown };
}

export interface HeadlessBackendProcessOptions {
  readonly args?: readonly string[];
  readonly env?: Readonly<Record<string, string | undefined>>;
  readonly io?: ProcessIo;
}

/** Run the supervised child entry point with injectable process boundaries. */
export async function runHeadlessBackendProcess(
  options: HeadlessBackendProcessOptions = {},
): Promise<number> {
  const args = options.args ?? process.argv.slice(2);
  const env = options.env ?? process.env;
  const io = options.io ?? {
    stdin: process.stdin as unknown as HeadlessBackendInput,
    stdout: process.stdout,
    stderr: process.stderr,
  };

  try {
    const launch = parseHeadlessBackendArgs(args);
    const backend = createHeadlessBackend({ stateRoot: launch.stateRoot, env });
    const result = await backend.run({
      stdin: io.stdin,
      stdout: line => { io.stdout.write(`${line}\n`); },
      stderr: line => { io.stderr.write(`${line}\n`); },
    });
    return result.exitCode;
  } catch (error) {
    const message = error instanceof HeadlessBackendConfigurationError
      ? "backend startup configuration is invalid"
      : "backend process failed";
    try {
      io.stderr.write(`${message}\n`);
    } catch {
      // There is nowhere safe to report an output failure.
    }
    return 1;
  }
}

if (import.meta.main) {
  process.exitCode = await runHeadlessBackendProcess();
}
