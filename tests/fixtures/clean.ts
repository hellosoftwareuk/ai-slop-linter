export interface OrderLine {
  unitPrice: number;
  quantity: number;
}

export interface Order {
  id: string;
  lines: OrderLine[];
}

export function calculateOrderTotal(order: Order): number {
  return order.lines.reduce(
    (total, line) => total + line.unitPrice * line.quantity,
    0,
  );
}

export function findExpensiveOrders(
  orders: Order[],
  minimumTotal: number,
): Order[] {
  return orders.filter((order) => calculateOrderTotal(order) >= minimumTotal);
}

