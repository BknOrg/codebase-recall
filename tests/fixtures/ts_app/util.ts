export function greet(name: string): string {
    return decorate(name);
}

function decorate(name: string): string {
    return `hello, ${name}`;
}
