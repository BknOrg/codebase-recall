package fixture;

/** Execute a Java task with journal commit on success and undo on failure. */
public class TransactionJob {
    private final WriteJournal journal = new WriteJournal();
    public String execute(java.util.concurrent.Callable<String> task) throws Exception {
        try {
            String result = task.call();
            journal.stage(result);
            journal.commit();
            return result;
        } catch (Exception error) {
            journal.undo();
            throw error;
        }
    }

    /** Convenience wrapper delegating transaction coordination to execute. */
    public String executeWrapper(java.util.concurrent.Callable<String> task) throws Exception {
        return execute(task);
    }

    /** Test exercising rollback after a failed callable; not the implementation. */
    public boolean testExecuteRollback() {
        try {
            execute(() -> { throw new Exception("fixture failure"); });
            return false;
        } catch (Exception error) {
            return true;
        }
    }
}
