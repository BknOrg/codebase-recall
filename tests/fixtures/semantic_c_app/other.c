static int local_helper(int value) { return value + 100; }
int other(void) { return local_helper(2); }
