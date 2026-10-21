// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * End-to-end tests for the WebUI commerce demo.
 *
 * Tests SSR (direct page loads), client-side navigation, sort filtering,
 * category switching, product pages, and visual regression screenshots.
 *
 * Start the server before running:
 *   cd examples/app/commerce && cargo run -p marketplace-api --release -- --port 3004
 */

import { test, expect, type Page } from '@playwright/test';

// 1×1 grey PNG placeholder — intercepts all remote image requests for
// deterministic screenshots without network dependency.
const PLACEHOLDER_PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==',
  'base64',
);

// Mock all image proxy requests so tests are hermetic (no resize overhead)
test.beforeEach(async ({ page }) => {
  await page.route('**/_image/**', (route) =>
    route.fulfill({ contentType: 'image/png', body: PLACEHOLDER_PNG }),
  );
});

async function expectSoftNavigation(
  page: Page,
  action: () => Promise<unknown>,
): Promise<void> {
  await page.evaluate(() => {
    (window as Window & { webuiSoftNavigationSentinel?: boolean })
      .webuiSoftNavigationSentinel = true;
  });
  await action();
  await expect.poll(() => page.evaluate(
    () => (window as Window & { webuiSoftNavigationSentinel?: boolean })
      .webuiSoftNavigationSentinel,
  )).toBe(true);
}

// ── SSR Tests (direct page loads) ────────────────────────────────

test.describe('SSR pages', () => {
  test('home page renders product grid', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('h3').first()).toBeVisible();
    const cards = page.locator('mp-product-card');
    await expect(cards.first()).toBeVisible();
    const count = await cards.count();
    expect(count).toBeGreaterThanOrEqual(6);
  });

  test('search page renders all products', async ({ page }) => {
    await page.goto('/search');
    await expect(page.getByRole('heading', { name: 'Collections' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Sort by' })).toBeVisible();
    const cards = page.locator('mp-product-card');
    await expect(cards.first()).toBeVisible();
    const count = await cards.count();
    expect(count).toBeGreaterThanOrEqual(10);
  });

  test('scriptless containers use dormant hosts while authored children hydrate', async ({ page }) => {
    await page.goto('/search/shirts');
    await expect(page.locator('mp-product-grid mp-product-card')).toHaveCount(3);

    const componentState = async (selector: string) =>
      page.locator(selector).first().evaluate((el) => {
        const component = el as HTMLElement & { $ready?: boolean; setState?: unknown };

        return {
          ready: component.$ready === true,
          setState: typeof component.setState === 'function',
        };
      });

    await expect(componentState('mp-page-search')).resolves.toEqual({ ready: true, setState: true });
    await expect(componentState('mp-product-grid')).resolves.toEqual({ ready: true, setState: true });
    await expect(componentState('mp-product-card')).resolves.toEqual({ ready: true, setState: true });
    await expect(componentState('mp-price')).resolves.toEqual({ ready: true, setState: true });

    const price = page.locator('mp-price').first();
    await price.evaluate((el) => {
      const component = el as HTMLElement & { setState(state: unknown): void };
      component.setState({ value: '$99.00', currencyCode: 'TEST' });
    });
    await expect(price.locator('.amount')).toHaveText('$99.00');
    await expect(price.locator('.currency')).toHaveText('TEST');
  });

  test('category page renders filtered products', async ({ page }) => {
    await page.goto('/search/shirts');
    await expect(page.getByRole('heading', { name: 'Collections' })).toBeVisible();
    // Shirts category has 3 products
    const cards = page.locator('mp-product-card');
    await expect(cards).toHaveCount(3);
    await expect(page.getByRole('heading', { name: 'Acme Circles T-Shirt' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Acme Prism T-Shirt' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Acme T-Shirt' })).toBeVisible();
  });

  test('category page has correct sort option URLs', async ({ page }) => {
    await page.goto('/search/shirts');
    const sortLinks = page.locator('mp-filter-list').getByRole('link');
    await expect(sortLinks.first()).toBeVisible();
    // All sort links should include /shirts
    for (const link of await sortLinks.all()) {
      const href = await link.getAttribute('href');
      expect(href).toContain('/search/shirts');
    }
  });

  test('product page renders product details', async ({ page }) => {
    await page.goto('/product/acme-geometric-circles-t-shirt');
    await expect(page.getByRole('heading', { name: 'Acme Circles T-Shirt', level: 1 })).toBeVisible();
    // Variant selectors
    await expect(page.getByText('COLOR')).toBeVisible();
    await expect(page.getByText('SIZE')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Black' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'XS' })).toBeVisible();
    // Add to cart
    await expect(page.getByRole('button', { name: 'Add To Cart' })).toBeVisible();
    // Related products
    await expect(page.getByRole('heading', { name: 'Related Products' })).toBeVisible();
  });

  test('about page renders content', async ({ page }) => {
    await page.goto('/about');
    await expect(page.locator('main')).toContainText('About');
  });

  test('category nav shows all categories on search page', async ({ page }) => {
    await page.goto('/search/shirts');
    await expect(page.getByRole('heading', { name: 'Collections' })).toBeVisible();
    const expectedCategories = ['All', 'Bags', 'Drinkware', 'Electronics',
      'Footware', 'Headwear', 'Hoodies', 'Jackets', 'Kids', 'Pets', 'Stickers'];
    for (const cat of expectedCategories) {
      await expect(page.locator('mp-category-nav').getByRole('link', { name: cat, exact: true }).first()).toBeVisible();
    }
  });

  test('sort options render on search page', async ({ page }) => {
    await page.goto('/search/shirts');
    await expect(page.getByRole('heading', { name: 'Sort by' })).toBeVisible();
    await expect(page.locator('mp-filter-list').getByRole('link', { name: 'Trending' }).first()).toBeVisible();
    await expect(page.locator('mp-filter-list').getByRole('link', { name: 'Latest arrivals' }).first()).toBeVisible();
    await expect(page.locator('mp-filter-list').getByRole('link', { name: 'Price: Low to high' }).first()).toBeVisible();
    await expect(page.locator('mp-filter-list').getByRole('link', { name: 'Price: High to low' }).first()).toBeVisible();
  });
});

// ── Client-side navigation ───────────────────────────────────────

test.describe('client-side navigation', () => {
  test('home → search category via navbar', async ({ page }) => {
    await page.goto('/');
    await page.locator('mp-navbar').getByRole('link', { name: 'Shirts' }).click();
    await expect(page).toHaveURL('/search/shirts');
    await expect(page.getByRole('heading', { name: 'Collections' })).toBeVisible();
    await expect(page.locator('mp-product-grid mp-product-card')).toHaveCount(3);
  });

  test('search → product → back to search', async ({ page }) => {
    await page.goto('/search/shirts');
    await expect(page.locator('mp-product-grid mp-product-card').first()).toBeVisible();

    // Click first product
    await page.locator('mp-product-card').filter({ hasText: 'Acme Circles T-Shirt' }).first().click();
    await expect(page).toHaveURL('/product/acme-geometric-circles-t-shirt');
    await expect(page.getByRole('heading', { name: 'Acme Circles T-Shirt', level: 1 })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Add To Cart' })).toBeVisible();

    // Navigate back to Shirts via navbar
    await page.locator('mp-navbar').getByRole('link', { name: 'Shirts' }).click();
    await expect(page).toHaveURL('/search/shirts');
    await expect(page.getByRole('heading', { name: 'Collections' })).toBeVisible();
    await expect(page.locator('mp-product-grid mp-product-card')).toHaveCount(3);
  });

  test('product page renders gallery and variants via client nav', async ({ page }) => {
    const pageErrors: string[] = [];
    page.on('pageerror', (error) => {
      pageErrors.push(error.message);
    });

    await page.goto('/');
    await page.locator('mp-product-card').filter({ hasText: 'Acme Circles T-Shirt' }).first().click();
    await expect(page).toHaveURL('/product/acme-geometric-circles-t-shirt');
    await expect(page.getByRole('heading', { name: 'Acme Circles T-Shirt', level: 1 })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Previous image' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Next image' })).toBeVisible();
    await expect(page.locator('mp-product-gallery .nav-btn svg')).toHaveCount(2);
    await expect(page.getByText('COLOR')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Black' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Related Products' })).toBeVisible();
    expect(pageErrors).toEqual([]);
  });

  test('add to cart opens panel with cart item', async ({ page }) => {
    await page.goto('/product/acme-geometric-circles-t-shirt');
    await page.getByRole('button', { name: 'Add To Cart' }).click();
    const cartPanel = page.locator('mp-cart-panel');
    const cartButton = page.locator('mp-navbar .cart-btn');
    await expect(cartPanel.getByText('My Cart')).toBeVisible();
    await expect(cartPanel.getByText('Your cart is empty.')).toHaveCount(0);
    const cartLine = cartPanel.locator('.cart-line');
    await expect(cartLine.getByRole('link', { name: 'Acme Circles T-Shirt' })).toBeVisible();
    await expect(cartLine.locator('.qty-count')).toHaveText('1');
    await expect(cartButton.locator('.cart-badge')).toHaveText('1');
  });

  test('opening empty cart shows empty state', async ({ page }) => {
    await page.goto('/');
    const cartButton = page.locator('mp-navbar .cart-btn');
    await cartButton.click();

    const cartPanel = page.locator('mp-cart-panel');
    await expect(cartPanel.getByText('My Cart')).toBeVisible();
    await expect(cartPanel.getByText('Your cart is empty.')).toBeVisible();
    await expect(cartPanel.locator('.cart-line')).toHaveCount(0);
    await expect(cartButton.locator('.cart-badge')).toHaveCount(0);
  });

  test('closing cart keeps ACME logo navigation pointed at home', async ({ page }) => {
    await page.goto('/product/acme-pacifier');
    const logo = page.locator('mp-navbar .logo-link');
    const cartButton = page.locator('mp-navbar .cart-btn');

    await expect(logo).toHaveAttribute('href', './');
    await cartButton.click();
    await page.locator('mp-cart-panel .close-btn').click();
    await expect(logo).toHaveAttribute('href', './');

    await expectSoftNavigation(page, () => logo.click());
    await expect(page).toHaveURL('/');
  });

  test('scriptless search route uses soft navigation', async ({ page }) => {
    await page.goto('/');
    await page.evaluate(() => { (window as any).__navTestMarker = Date.now(); });

    await expectSoftNavigation(
      page,
      () => page.locator('mp-navbar').getByRole('link', { name: 'Shirts' }).click(),
    );
    await expect(page.getByRole('heading', { name: 'Collections' })).toBeVisible();

    const marker = await page.evaluate(() => (window as any).__navTestMarker);
    expect(typeof marker).toBe('number');
  });
});

// ── Category switching ───────────────────────────────────────────

test.describe('category switching', () => {
  test('switch between categories via sidebar', async ({ page }) => {
    await page.goto('/search/shirts');
    await expect(page.locator('mp-product-card')).toHaveCount(3);

    // Switch to Stickers
    await expectSoftNavigation(
      page,
      () => page.locator('mp-category-nav').getByRole('link', { name: 'Stickers' }).click(),
    );
    await expect(page).toHaveURL('/search/stickers');
    await expect(page.locator('mp-product-card')).toHaveCount(2);
    await expect(page.getByRole('heading', { name: 'Acme Sticker' })).toBeVisible();
  });

  test('sort options update when switching categories', async ({ page }) => {
    await page.goto('/search/stickers');
    // Sort links should point to /search/stickers
    let sortLink = page.locator('mp-filter-list').getByRole('link', { name: 'Price: High to low' });
    await expect(sortLink).toHaveAttribute('href', /\/search\/stickers\?sort=/);

    // Switch to Shirts via sidebar
    await expectSoftNavigation(
      page,
      () => page.locator('mp-category-nav').getByRole('link', { name: 'Shirts' }).click(),
    );
    await expect(page).toHaveURL('/search/shirts');

    // Sort links should now point to /search/shirts
    sortLink = page.locator('mp-filter-list').getByRole('link', { name: 'Price: High to low' });
    await expect(sortLink).toHaveAttribute('href', /\/search\/shirts\?sort=/);
  });
});

// ── Sort filtering ───────────────────────────────────────────────

test.describe('sort filtering', () => {
  test('sort by price high to low on SSR page', async ({ page }) => {
    await page.goto('/search/shirts');
    await expect(page.locator('mp-product-card')).toHaveCount(3);

    await expectSoftNavigation(
      page,
      () => page.locator('mp-filter-list').getByRole('link', { name: 'Price: High to low' }).click(),
    );
    await expect(page).toHaveURL('/search/shirts?sort=price-desc');

    // Prism ($25) should be first
    const firstCard = page.locator('mp-product-card').first();
    await expect(firstCard).toContainText('Acme Prism T-Shirt');
    await expect(firstCard).toContainText('$25.00');
  });

  test('sort by price low to high', async ({ page }) => {
    await page.goto('/search/shirts');
    await expectSoftNavigation(
      page,
      () => page.locator('mp-filter-list').getByRole('link', { name: 'Price: Low to high' }).click(),
    );
    await expect(page).toHaveURL('/search/shirts?sort=price-asc');

    // $20 shirts should come before $25
    const firstCard = page.locator('mp-product-card').first();
    await expect(firstCard).toContainText('$20.00');
  });

  test('sort preserves category in URL', async ({ page }) => {
    await page.goto('/search/headwear');
    await expectSoftNavigation(
      page,
      () => page.locator('mp-filter-list').getByRole('link', { name: 'Price: High to low' }).click(),
    );
    await expect(page).toHaveURL('/search/headwear?sort=price-desc');

    // Cowboy Hat ($160) should be first
    const firstCard = page.locator('mp-product-card').first();
    await expect(firstCard).toContainText('Acme Cowboy Hat');
    await expect(firstCard).toContainText('$160.00');
  });

  test('sort after category soft navigation', async ({ page }) => {
    await page.goto('/');

    // Navigate to Stickers
    await expectSoftNavigation(
      page,
      () => page.locator('mp-navbar').getByRole('link', { name: 'Stickers' }).click(),
    );
    await expect(page).toHaveURL('/search/stickers');
    await expect(page.locator('mp-product-grid mp-product-card')).toHaveCount(2);

    // Switch to Shirts via sidebar
    await expectSoftNavigation(
      page,
      () => page.locator('mp-category-nav').getByRole('link', { name: 'Shirts' }).first().click(),
    );
    await expect(page).toHaveURL('/search/shirts');
    await expect(
      page.locator('mp-filter-list a[href*="/search/shirts?sort=price-desc"]'),
    ).not.toHaveCount(0);

    // Sort by price high to low
    await expectSoftNavigation(
      page,
      () => page.locator('mp-filter-list').getByRole('link', { name: 'Price: High to low' }).first().click(),
    );
    await expect(page).toHaveURL('/search/shirts?sort=price-desc');

    // Prism ($25) should be first
    const firstCard = page.locator('mp-product-grid mp-product-card').first();
    await expect(firstCard).toContainText('Acme Prism T-Shirt');
  });

  test('active sort indicator updates', async ({ page }) => {
    await page.goto('/search/shirts');
    // "Price: High to low" is a link
    const sortNav = page.locator('mp-filter-list');
    await expect(sortNav.getByRole('link', { name: 'Price: High to low' }).first()).toBeVisible();

    await expectSoftNavigation(
      page,
      () => sortNav.getByRole('link', { name: 'Price: High to low' }).first().click(),
    );
    await expect(page).toHaveURL(/sort=price-desc/);

    // Now "Relevance" should be a link
    await expect(sortNav.getByRole('link', { name: 'Relevance' }).first()).toBeVisible();
  });
});

// ── Screenshot tests ─────────────────────────────────────────────

// ── Extended client navigation tests ─────────────────────────────

test.describe('category navigation flows', () => {
  test('navigate through multiple categories via sidebar', async ({ page }) => {
    await page.goto('/search/shirts');
    await expect(page.locator('mp-product-grid mp-product-card')).toHaveCount(3);

    // Shirts → Headwear
    await expectSoftNavigation(
      page,
      () => page.locator('mp-category-nav').getByRole('link', { name: 'Headwear' }).first().click(),
    );
    await expect(page).toHaveURL('/search/headwear');
    await expect(page.locator('mp-product-grid mp-product-card')).toHaveCount(3);
    await expect(page.locator('mp-product-grid')).toContainText('Acme Cowboy Hat');

    // Headwear → Stickers
    await expectSoftNavigation(
      page,
      () => page.locator('mp-category-nav').getByRole('link', { name: 'Stickers' }).first().click(),
    );
    await expect(page).toHaveURL('/search/stickers');
    await expect(page.locator('mp-product-grid mp-product-card')).toHaveCount(2);

    // Stickers → All
    await expectSoftNavigation(
      page,
      () => page.locator('mp-category-nav').getByRole('link', { name: 'All' }).first().click(),
    );
    await expect(page).toHaveURL('/search');
    await expect.poll(() => page.locator('mp-product-grid mp-product-card').count()).toBeGreaterThanOrEqual(2);
  });

  test('category active state updates in sidebar', async ({ page }) => {
    await page.goto('/search/shirts');
    // "Shirts" should not be a link (it's active)
    await expect(page.locator('mp-category-nav').getByRole('link', { name: 'Shirts' })).toHaveCount(0);

    // Click Stickers
    await expectSoftNavigation(
      page,
      () => page.locator('mp-category-nav').getByRole('link', { name: 'Stickers' }).first().click(),
    );
    await expect(page).toHaveURL('/search/stickers');
    // Now "Stickers" should not be a link, and "Shirts" should be
    await expect(page.locator('mp-category-nav').getByRole('link', { name: 'Stickers' })).toHaveCount(0);
    await expect(page.locator('mp-category-nav').getByRole('link', { name: 'Shirts' }).first()).toBeVisible();
  });

  test('search results page → category via sidebar (hydration parity)', async ({ page }) => {
    // Regression: boolean attr hydration mismatch left orphaned "All" node
    // when navigating from a search-results page to a category page.
    await page.goto('/search?q=test');
    await expect(page.locator('mp-category-nav')).toBeVisible();

    // "All" should be active (not a link) on the search results page
    const nav = page.locator('mp-category-nav');
    await expect(nav.getByRole('link', { name: 'All', exact: true })).toHaveCount(0);

    // Navigate to Shirts via sidebar
    await expectSoftNavigation(
      page,
      () => nav.getByRole('link', { name: 'Shirts' }).first().click(),
    );
    await expect(page).toHaveURL('/search/shirts');

    // "All" must become a link (no longer active) — no duplicate "All" text
    await expect(nav.getByRole('link', { name: 'All', exact: true }).first()).toBeVisible();
    // "Shirts" should now be active (not a link)
    await expect(nav.getByRole('link', { name: 'Shirts' })).toHaveCount(0);
    // Product grid must show the 3 shirts (validates repeat hydration fix)
    await expect(page.locator('mp-product-grid mp-product-card')).toHaveCount(3);
  });

  test('search → category → product → different category', async ({ page }) => {
    await page.goto('/search');

    // Go to Headwear
    await expectSoftNavigation(
      page,
      () => page.locator('mp-category-nav').getByRole('link', { name: 'Headwear' }).first().click(),
    );
    await expect(page).toHaveURL('/search/headwear');
    await expect(page.locator('mp-product-grid')).toContainText('Acme Cowboy Hat');

    // Click a product
    await page.locator('mp-product-card').filter({ hasText: 'Acme Cowboy Hat' }).first().click();
    await expect(page).toHaveURL('/product/acme-cowboy-hat');
    await expect(page.getByRole('heading', { name: 'Acme Cowboy Hat', level: 1 })).toBeVisible();

    // Go to Shirts from navbar
    await expectSoftNavigation(
      page,
      () => page.locator('mp-navbar').getByRole('link', { name: 'Shirts' }).click(),
    );
    await expect(page).toHaveURL('/search/shirts');
    await expect(page.locator('mp-product-grid mp-product-card')).toHaveCount(3);
  });
});

test.describe('sort and category combined', () => {
  test('sort then switch category resets sort', async ({ page }) => {
    await page.goto('/search/shirts');

    // Sort by price desc
    await expectSoftNavigation(
      page,
      () => page.locator('mp-filter-list').getByRole('link', { name: 'Price: High to low' }).first().click(),
    );
    await expect(page).toHaveURL('/search/shirts?sort=price-desc');
    await expect(page.locator('mp-product-grid mp-product-card').first()).toContainText('Acme Prism T-Shirt');

    // Switch to Headwear — sort should reset (no ?sort= in URL)
    await expectSoftNavigation(
      page,
      () => page.locator('mp-category-nav').getByRole('link', { name: 'Headwear' }).first().click(),
    );
    await expect(page).toHaveURL('/search/headwear');
    // Sort links should point to /search/headwear
    await expect(page.locator('mp-filter-list').getByRole('link', { name: 'Price: High to low' }).first())
      .toHaveAttribute('href', /\/search\/headwear\?sort=/);
  });

  test('sort multiple times in same category', async ({ page }) => {
    await page.goto('/search/headwear');

    // Sort price high → low
    await expectSoftNavigation(
      page,
      () => page.locator('mp-filter-list').getByRole('link', { name: 'Price: High to low' }).first().click(),
    );
    await expect(page).toHaveURL('/search/headwear?sort=price-desc');
    await expect(page.locator('mp-product-grid mp-product-card').first()).toContainText('Acme Cowboy Hat');
    await expect(page.locator('mp-product-grid mp-product-card').first()).toContainText('$160.00');

    // Sort price low → high
    await expectSoftNavigation(
      page,
      () => page.locator('mp-filter-list').getByRole('link', { name: 'Price: Low to high' }).first().click(),
    );
    await expect(page).toHaveURL('/search/headwear?sort=price-asc');
    await expect(page.locator('mp-product-grid mp-product-card').first()).toContainText('Acme Baby Cap');
    await expect(page.locator('mp-product-grid mp-product-card').first()).toContainText('$10.00');

    // Sort by relevance
    await expectSoftNavigation(
      page,
      () => page.locator('mp-filter-list').getByRole('link', { name: 'Relevance' }).first().click(),
    );
    await expect(page).toHaveURL('/search/headwear?sort=relevance');
  });

  test('sort high-to-low across categories SSR', async ({ page }) => {
    // Verify server-side sort on different categories
    await page.goto('/search/headwear?sort=price-desc');
    await expect(page.locator('mp-product-grid mp-product-card').first()).toContainText('Acme Cowboy Hat');

    await page.goto('/search/shirts?sort=price-desc');
    await expect(page.locator('mp-product-grid mp-product-card').first()).toContainText('Acme Prism T-Shirt');
  });
});

test.describe('search', () => {
  test('search form submits to /search with query', async ({ page }) => {
    await page.goto('/');
    const searchInput = page.locator('mp-search-bar input[name="q"]').first();
    await searchInput.fill('mug');
    await expectSoftNavigation(page, () => searchInput.press('Enter'));
    await expect(page).toHaveURL(/\/search\?q=mug/);
  });

  test('search form submit shows no-results message for invalid query', async ({ page }) => {
    await page.goto('/');
    const searchInput = page.locator('mp-search-bar input[name="q"]').first();
    await searchInput.fill('testsdsd');
    await expectSoftNavigation(page, () => searchInput.press('Enter'));
    await expect(page).toHaveURL(/\/search\?q=testsdsd/);
    await expect(page.locator('mp-product-grid mp-product-card')).toHaveCount(0);
    await expect(page.locator('mp-product-grid')).toContainText('There are no products that match');
    await expect(page.locator('mp-product-grid')).toContainText('testsdsd');
  });

  test('search results show matching products', async ({ page }) => {
    await page.goto('/search?q=sticker');
    await expect(page.locator('mp-product-grid mp-product-card').first()).toBeVisible();
    const cards = page.locator('mp-product-grid mp-product-card');
    const count = await cards.count();
    expect(count).toBeGreaterThanOrEqual(1);
  });

  test('search from product page navigates to results', async ({ page }) => {
    await page.goto('/product/acme-geometric-circles-t-shirt');
    await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
    const searchInput = page.locator('mp-search-bar input[name="q"]').first();
    await searchInput.fill('hoodie');
    await expectSoftNavigation(page, () => searchInput.press('Enter'));
    await expect(page).toHaveURL(/\/search\?q=hoodie/);
  });

  test('empty search shows no-results message', async ({ page }) => {
    await page.goto('/search?q=xyznonexistent');
    await expect(page.locator('mp-product-grid mp-product-card')).toHaveCount(0);
    await expect(page.locator('mp-product-grid')).toContainText('no products');
  });

  test('empty search shows no-results in SSR (no JS)', async ({ browser }) => {
    const context = await browser.newContext({ javaScriptEnabled: false });
    const page = await context.newPage();
    await page.goto('/search?q=xyznonexistent');
    await expect(page.locator('mp-product-grid')).toContainText('no products');
    await context.close();
  });
});

test.describe('product navigation', () => {
  test('navigate between products via related', async ({ page }) => {
    await page.goto('/product/acme-geometric-circles-t-shirt');
    await expect(page.getByRole('heading', { name: 'Acme Circles T-Shirt', level: 1 })).toBeVisible();

    // Click a related product
    await page.locator('mp-product-card').filter({ hasText: 'Acme Cap' }).first().click();
    await expect(page).toHaveURL('/product/acme-cap');
    await expect(page.getByRole('heading', { name: 'Acme Cap', level: 1 })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Add To Cart' })).toBeVisible();
  });

  test('product page has correct variant buttons', async ({ page }) => {
    await page.goto('/product/acme-t-shirt');
    await expect(page.getByRole('heading', { name: 'Acme T-Shirt', level: 1 })).toBeVisible();
    // Colors
    for (const color of ['Black', 'Blue', 'Gray', 'Pink', 'White']) {
      await expect(page.getByRole('button', { name: color })).toBeVisible();
    }
    // Sizes
    for (const size of ['XS', 'S', 'M', 'L', 'XL', 'XXL', 'XXXL']) {
      await expect(page.getByRole('button', { name: size, exact: true })).toBeVisible();
    }
  });
});

// ── Regression: variant selector after SSR refresh (#175) ────────

test.describe('variant selector SSR hydration', () => {
  test('clicking variant updates active state after page refresh', async ({ page }) => {
    // SSR load — variant selector is hydrated from server-rendered HTML
    await page.goto('/product/acme-geometric-circles-t-shirt');
    await expect(page.getByRole('button', { name: 'Black' })).toBeVisible();

    // Click "Blue" — should get active attribute
    await page.getByRole('button', { name: 'Blue' }).click();

    // Verify "Blue" is now active and "Black" is not
    await expect(page.getByRole('button', { name: 'Blue' })).toHaveAttribute('active', '');
    await expect(page.getByRole('button', { name: 'Black' })).not.toHaveAttribute('active', '');
  });

  test('clicking size variant updates active state after page refresh', async ({ page }) => {
    await page.goto('/product/acme-t-shirt');
    await expect(page.getByRole('button', { name: 'M', exact: true })).toBeVisible();

    await page.getByRole('button', { name: 'M', exact: true }).click();

    await expect(page.getByRole('button', { name: 'M', exact: true })).toHaveAttribute('active', '');
    await expect(page.getByRole('button', { name: 'XS' })).not.toHaveAttribute('active', '');
  });
});

// ── Regression: add-to-cart duplicate for items (#176) ───────────

test.describe('cart item deduplication', () => {
  test('SSR-hydrated cart does not duplicate for-loop items', async ({ page }) => {
    // Step 1: add an item to cart (sets the cart cookie)
    await page.goto('/product/acme-geometric-circles-t-shirt');
    await page.getByRole('button', { name: 'Add To Cart' }).click();
    const cartPanel = page.locator('mp-cart-panel');
    await expect(cartPanel.getByText('My Cart')).toBeVisible();
    await expect(cartPanel.locator('.cart-line')).toHaveCount(1);

    // Step 2: reload the page — server now renders cart items in SSR HTML
    await page.reload();
    await expect(page.getByRole('heading', { name: 'Acme Circles T-Shirt', level: 1 })).toBeVisible();

    // Step 3: open the cart and verify no duplicates after hydration
    await page.locator('mp-navbar .cart-btn').click();
    await expect(cartPanel.getByText('My Cart')).toBeVisible();
    await expect(cartPanel.locator('.cart-line')).toHaveCount(1);
    await expect(cartPanel.locator('.cart-line').getByRole('link', { name: 'Acme Circles T-Shirt' })).toBeVisible();
  });
});

// ── Screenshot tests ─────────────────────────────────────────────

test.describe('visual regression', () => {
  test('home page screenshot', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('mp-product-card').first()).toBeVisible();
    await expect(page).toHaveScreenshot('home-page.png', { maxDiffPixelRatio: 0.01 });
  });

  test('search shirts screenshot', async ({ page }) => {
    await page.goto('/search/shirts');
    await expect(page.locator('mp-product-card')).toHaveCount(3);
    await expect(page).toHaveScreenshot('search-shirts.png', { maxDiffPixelRatio: 0.01 });
  });

  test('search shirts sorted screenshot', async ({ page }) => {
    await page.goto('/search/shirts');
    await expectSoftNavigation(
      page,
      () => page.locator('mp-filter-list').getByRole('link', { name: 'Price: High to low' }).click(),
    );
    await expect(page).toHaveURL(/sort=price-desc/);
    await expect(page.locator('mp-product-card').first()).toContainText('Acme Prism T-Shirt');
    await expect(page).toHaveScreenshot('search-shirts-sorted.png', { maxDiffPixelRatio: 0.01 });
  });

  test('product page screenshot', async ({ page }) => {
    await page.goto('/product/acme-geometric-circles-t-shirt');
    await expect(page.getByRole('heading', { name: 'Acme Circles T-Shirt', level: 1 })).toBeVisible();
    await expect(page).toHaveScreenshot('product-circles-tshirt.png', { maxDiffPixelRatio: 0.01 });
  });

  test('product page via client nav screenshot', async ({ page }) => {
    await page.goto('/search/shirts');
    await expect(page.locator('mp-product-card').first()).toBeVisible();
    await page.locator('mp-product-card').filter({ hasText: 'Acme Circles T-Shirt' }).first().click();
    await expect(page.getByRole('heading', { name: 'Acme Circles T-Shirt', level: 1 })).toBeVisible();
    await expect(page).toHaveScreenshot('product-via-client-nav.png', { maxDiffPixelRatio: 0.01 });
  });

  test('search all products screenshot', async ({ page }) => {
    await page.goto('/search');
    await expect(page.locator('mp-product-card').first()).toBeVisible();
    await expect(page).toHaveScreenshot('search-all.png', { maxDiffPixelRatio: 0.01 });
  });
});

// ── Mobile screenshot tests ──────────────────────────────────────

test.describe('mobile layout', () => {
  test('search page mobile screenshot', async ({ page }) => {
    await page.goto('/search');
    await expect(page.locator('mp-product-card').first()).toBeVisible();
    await expect(page).toHaveScreenshot('mobile-search.png', { maxDiffPixelRatio: 0.01 });
  });

  test('category page mobile screenshot', async ({ page }) => {
    await page.goto('/search/shirts');
    await expect(page.locator('mp-product-card')).toHaveCount(3);
    await expect(page).toHaveScreenshot('mobile-search-shirts.png', { maxDiffPixelRatio: 0.01 });
  });

  test('category switch via client nav mobile screenshot', async ({ page }) => {
    await page.goto('/search');
    await expect(page.locator('mp-product-card').first()).toBeVisible();
    // On mobile the category nav is a <details> dropdown — open it first
    await page.locator('mp-category-nav').locator('summary').click();
    await expectSoftNavigation(
      page,
      () => page.locator('mp-category-nav').getByRole('link', { name: 'Shirts' }).click(),
    );
    await expect(page).toHaveURL(/\/search\/shirts/);
    await expect(page.locator('mp-product-card')).toHaveCount(3);
    await expect(page).toHaveScreenshot('mobile-category-switch.png', { maxDiffPixelRatio: 0.01 });
  });

  test('product page mobile screenshot', async ({ page }) => {
    await page.goto('/product/acme-geometric-circles-t-shirt');
    await expect(page.getByRole('heading', { name: 'Acme Circles T-Shirt', level: 1 })).toBeVisible();
    await expect(page).toHaveScreenshot('mobile-product.png', { maxDiffPixelRatio: 0.01 });
  });
});
