import { UserStore, OrderStore } from "./store";

export class Service {
  private users: UserStore;
  private orders: OrderStore;

  constructor() {
    this.users = new UserStore();
    this.orders = new OrderStore();
  }

  lookup(id: string): void {
    // `findById` is defined on BOTH stores. The declared field types
    // `users: UserStore` / `orders: OrderStore` make each call resolve
    // to the right class.
    this.users.findById(id);
    this.orders.findById(id);
  }
}
