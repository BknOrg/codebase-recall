// These labels refer to declaration targets, including the selected overload.
int strong(){return 1;}
int choose(int value){return value;}
double choose(double value){return value;}
int run(){return strong()+choose(1);}
