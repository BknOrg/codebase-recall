export type ID = string;
export class Worker {
    value: number = 1;
    constructor(value: number) {}
    run(value?: string) {}
}
export const factory = (value: number) => value;
