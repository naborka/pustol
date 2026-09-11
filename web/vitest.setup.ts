import "@testing-library/dom";

/**
 * jsdom does no layout, so it implements no scrolling either.
 *
 * The sheet keeps a focused field in view by scrolling it inside its own panel; without this, any
 * test that opens a sheet with something focused in it fails on a missing method rather than on
 * anything about the app. Stubbed rather than guarded in the component, because a guard would make
 * the real browsers' behaviour conditional on an environment gap.
 */
if (typeof Element !== "undefined" && !Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = function scrollIntoView() {
    // Nothing to do: there is no viewport to scroll within.
  };
}
