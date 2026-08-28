import { liveValue } from "./state";
declare const first: boolean;
declare const second: boolean;
declare const value: string | null;

if (first) {
  returnFromSomewhere();
} else {
  const scoped = compute();
  use(scoped);
}

if (first) {
  work();
} else {
  /* branch rationale */
  workAgain();
}

function oneStep(ready: boolean): void {
  if (ready) {
    execute();
  }
}

function onlyConditional(ready: boolean): void {
  if (ready) {
    execute();
    report();
  }
}

function lexicalGuard(ready: boolean): void {
  if (ready) {
    const result = execute();
    use(result);
  }
}

function* generatorGuard(ready: boolean) {
  if (ready) {
    yield execute();
    report();
  }
}

function mutated(input: string): string {
  const alias = input;
  input = "changed";
  return alias;
}

function repeated(input: string): string {
  const alias = input;
  audit(alias);
  return alias;
}

function shorthand(input: string) {
  const alias = input;
  return { alias };
}

function shadowed(input: string): string {
  const alias = input;
  {
    const input = "other";
    return alias;
  }
}

function contracted(input: string): string {
  const alias: string = input;
  return alias;
}

function imported(): string {
  const alias = liveValue;
  return alias;
}

if (value !== null) {
  use(value);
} else if (typeof value === "string") {
  use(value);
}

if (first) {
  work();
} else if (second) {
  workAgain();
}

if (first) {
  work();
} else if (second) {
  work();
} else {
  fallback();
}
