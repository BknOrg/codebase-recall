export class Worker { constructor(value) { this.value = value; } run(value) { return value; } }
export const create = (value = 1) => new Worker(value);
