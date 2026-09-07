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
      <h1 className="mb-2 text-2xl font-bold">Catalogue</h1>
      <p className="mb-6 text-sm text-base-content/60">
        Five things, priced in XAF. Prices are integer minor units all the way
        to the rail — XAF is zero-decimal, so 12 000 FCFA is the integer 12000.
      </p>
      <ul className="grid grid-cols-[repeat(auto-fill,minmax(16rem,1fr))] gap-4">
        {products.map((product) => (
          <li key={product.id} className="card bg-base-100 shadow-sm">
            <div className="card-body gap-2">
              <h3 className="card-title text-base">{product.name}</h3>
              <p className="flex-1 text-sm text-base-content/60">
                {product.description}
              </p>
              <span className="font-semibold tabular-nums">
                {formatMinor(product.priceMinor, product.currency)}
              </span>
              <AddToCart productId={product.id} name={product.name} />
            </div>
          </li>
        ))}
      </ul>
    </>
  );
}
