import { AddToCart } from "@/components/add-to-cart";
import { formatMinor } from "@/money";
import { serverCaller } from "@/server/context";

// Rendered per request: the catalogue lives in Postgres, and a build with no
// database must not try to prerender it into the image.
export const dynamic = "force-dynamic";

export default async function CataloguePage() {
  const products = await serverCaller().products.list();
  return (
    <>
      <h1>Catalogue</h1>
      <p style={{ color: "var(--muted)" }}>
        Five things, priced in XAF. Prices are integer minor units all the way
        to the rail — XAF is zero-decimal, so 12 000 FCFA is the integer 12000.
      </p>
      <ul className="grid">
        {products.map((product) => (
          <li key={product.id} className="card">
            <h3>{product.name}</h3>
            <p>{product.description}</p>
            <span className="price">
              {formatMinor(product.priceMinor, product.currency)}
            </span>
            <AddToCart productId={product.id} name={product.name} />
          </li>
        ))}
      </ul>
    </>
  );
}
