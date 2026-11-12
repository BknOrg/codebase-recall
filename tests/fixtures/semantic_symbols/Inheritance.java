package sample;
class Parent { protected int value; public int run(int x) { return value; } }
class Child extends Parent { public int run(int x) { return x; } }
