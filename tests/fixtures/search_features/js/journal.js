let pending = [];
export function stageWrite(value) { pending.push(value); }
export function commitWrite() { const committed = pending; pending = []; return committed; }
export function rollbackWrite() { pending = []; }
