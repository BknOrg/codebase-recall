export class UserStore {
  findById(id: string): void {
    console.log("UserStore.findById", id);
  }
}

export class OrderStore {
  findById(id: string): void {
    console.log("OrderStore.findById", id);
  }
}
