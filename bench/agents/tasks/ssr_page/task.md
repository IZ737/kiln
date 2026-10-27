This program has a `shop` module with the products for sale, but no
pages yet. Add a server-rendered page at `/` with a glaze:

- a heading `Shop`, and a table with a row per product: its name, its
  price and its stock;
- prices are dollars and cents, from `cents`: `1250` is `$12.50`, `5` is
  `$0.05`. Put this in `shop` as `pub fn price(cents: Int) -> String`;
- a product with no stock shows `Sold out` instead of its stock;
- `/?q=TEXT` shows only the products whose name contains TEXT, ignoring
  case, and says `No products match.` when none do.

Names are text from a database in a real shop, so the page must escape
them.
