# korekore website

A one-page site for korekore, built from the same design system and pixel data as the apps and the box ([`docs/design/korekore/`](../../docs/design/korekore/README.md)).

| File | What |
| --- | --- |
| `index.template.html` | The page: copy, styles and script, with `{{…}}` placeholders where pixel art goes |
| `build.py` | Fills the placeholders with inline SVG from `tools/korekore-pixel-art.py` and wraps the page into a standalone document |
| `index.html` | The built site: one file, no other assets, ready to host |

Edit the template, never `index.html`, then rebuild:

```sh
site/korekore/build.py
```

Placeholders: `{{kore:<expression>:<scale>[:gray]}}`, `{{chibi:<who>:<scale>}}`, `{{icon:<key>:<scale>}}`, `{{tag:<dot px>:<colour>[:grain]}}`.

## What is left before launch

### Payments

The buy block sits in the hero (`#buy`). Its settings are the `PRICING` object in the template's script:

- `launchLink` is the $59 Stripe payment link used on vibekeys.dev today (`https://buy.stripe.com/eVqbJ26z9dVNdNsbqd63K18`).
- `regularLink` is empty: create the $69 Stripe payment link and put it here. Until then, after the deadline the button sends people to vibekeys.dev instead of selling at $59.
- `launchEnds` is November 7, 11:59 pm PT (`2026-11-08T07:59:00Z`). vibekeys.dev notes that the $59 link should be deactivated in Stripe at that moment too, or links already shared keep selling at $59.
- **Add to cart** links to vibekeys.dev today. A real cart on this site means calling the vibekeys checkout API (`https://vibekeys-crm-production.up.railway.app/api/checkout`, the one `vibekeys.dev` uses through `assets/vk.js`): it must allow the korekore.ai origin (CORS) and the site needs the same cart logic and product id (`data-cart-add="buddy"`). Alternatively drop the cart button and keep the single Order button.
- Stripe's checkout shows the product as VibeBuddy; the small print under the button says so. Renaming the Stripe product is optional.

### Hosting

`index.html` is static and self-contained (fonts come from Google Fonts). Any static host works. `updates.korekore.ai` is already served by Cloudflare, so Cloudflare Pages on `korekore.ai` is the shortest path.

### Timing

The site shows the korekore UI (Kore as the default Character, the new screens and Characters), which is still being built ([ADR-0020](../../docs/adr/0020-the-box-and-the-apps-share-the-korekore-look.md)). Launch it together with that UI, or buyers will get a box that does not match the page.
