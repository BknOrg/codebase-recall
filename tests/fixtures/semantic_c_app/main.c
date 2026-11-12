// Unicode 日本語 keeps original byte positions.
#include "api.h"
static int local_helper(int value) { return value; }
int entry(void) { int value = 1; return local_helper(value); }
int api(int first, ...) { return local_helper(first); }
