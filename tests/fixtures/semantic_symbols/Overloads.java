package sample;
class Overloads {
    int choose(int value) { return value; }
    int choose(String value) { return 2; }
    int certain() { return choose(1); }
    int uncertain(Object value) { return choose(value); }
}
