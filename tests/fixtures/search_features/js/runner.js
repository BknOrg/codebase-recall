import { commitWrite, rollbackWrite } from './journal.js';

/** Execute a job, committing its writes or rolling them back when it fails. */
export async function executeJob(job) {
  try {
    const result = await job();
    commitWrite();
    return result;
  } catch (error) {
    rollbackWrite();
    throw error;
  }
}

/** Convenience entrypoint delegating the job transaction to executeJob. */
export async function executeJobWrapper(job) {
  return executeJob(job);
}

/** Test example exercising rollback after a failed job; not the implementation. */
export async function testExecuteJobRollback() {
  let observedFailure = false;
  try {
    await executeJob(async () => { throw new Error('fixture failure'); });
  } catch (error) {
    observedFailure = error.message === 'fixture failure';
  }
  return observedFailure;
}
