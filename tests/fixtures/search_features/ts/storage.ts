export class Storage {
  private pending: string[] = [];
  stage(value: string): void { this.pending.push(value); }
  commit(): string[] { const result = this.pending; this.pending = []; return result; }
  rollback(): void { this.pending = []; }
}
