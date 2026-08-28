function loadName(name: string): string {
  if (name.length === 0) {
    return "missing";
  } else {
    audit(name);
    return normalize(name);
  }
}

function runJob(ready: boolean): void {
  prepare();
  if (ready) {
    execute();
    report();
  }
}

function render(input: string): string {
  const currentInput = input;
  return format(currentInput);
}

function renderLocal(): string {
  const input = readInput();
  const currentInput = input;
  return format(currentInput);
}

function notification(primary: boolean, fallback: boolean): string | null {
  if (primary) {
    return "send";
  } else if (fallback) {
    return "send";
  }
  return null;
}
