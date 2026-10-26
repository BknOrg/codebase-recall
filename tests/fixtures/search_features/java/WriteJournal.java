package fixture;
public class WriteJournal {
    private String pending;
    public void stage(String value) { pending = value; }
    public void commit() { pending = null; }
    public void undo() { pending = null; }
}
