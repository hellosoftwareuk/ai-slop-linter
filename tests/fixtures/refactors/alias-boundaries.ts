function dynamic(input: string): string {
  const alias = input;
  eval("alias");
  return alias;
}

function documented(input: string): string {
  const alias = input; // preserve domain name
  return alias;
}
