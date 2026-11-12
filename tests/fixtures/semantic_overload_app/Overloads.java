package sample;
class Overloads {
    int choose(String value) { return 2; }
    int choose(int value) { return value; }
    int certain() { return choose(1); }
    int uncertain() { return choose(externalValue()); }
}
