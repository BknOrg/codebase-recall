#pragma once
namespace app {
using Count = int;
inline int helper(int value=1) { return value; }
template<class T> concept Number = requires(T value) { value + value; };
struct Base { virtual int run(int value=1) { return value; } };
class Child : public Base {
public:
  Child() {}
  ~Child() {}
  int first, second;
  int run(int value=2) override { return first; }
};
}
