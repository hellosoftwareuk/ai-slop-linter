interface GreetingProps {
  name: string;
  isReturning: boolean;
}

export function Greeting({ name, isReturning }: GreetingProps) {
  const salutation = isReturning ? "Welcome back" : "Welcome";
  return <h1>{salutation}, {name}</h1>;
}

