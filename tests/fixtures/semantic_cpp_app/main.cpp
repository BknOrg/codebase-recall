// Unicode 日本語
#include "api.h"
int start() { int local = 1; return app::helper(local); }
int same(int value) { return value; }
int same(double value) { return 2; }
int ambiguous() { return same(unknown()); }
