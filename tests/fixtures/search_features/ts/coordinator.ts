import { Storage } from './storage';

/** Coordinate a request's staged writes and undo them if the request fails. */
export class RequestCoordinator {
  constructor(private readonly storage: Storage) {}
  run(request: () => string): string {
    try {
      const result = request();
      this.storage.stage(result);
      this.storage.commit();
      return result;
    } catch (error) {
      this.storage.rollback();
      throw error;
    }
  }
}

/** Thin adapter; the coordinator owns the request transaction implementation. */
export function runRequestWrapper(coordinator: RequestCoordinator, request: () => string): string {
  return coordinator.run(request);
}

/** Test exercising the coordinator's failure branch; not production coordination. */
export function testCoordinatorRollback(coordinator: RequestCoordinator): boolean {
  try {
    coordinator.run(() => { throw new Error('fixture failure'); });
    return false;
  } catch {
    return true;
  }
}
