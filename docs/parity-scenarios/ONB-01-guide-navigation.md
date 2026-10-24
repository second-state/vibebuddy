# ONB-01: First-run guide: pages and navigation

Category: First-run guide · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/OnboardingView.swift`
- `app/Sources/VibeBuddy/AppDelegate.swift` (`showOnboarding`)

## Initial state (input)

- First launch (LIFE-01).

## Trigger

The user moves through the guide.

## Workflow (Mac app)

1. Pages: Welcome › Find the box › Connect agents › Pick a character › Wi-Fi › Launch at login › Done, with the current page highlighted.
2. Back on every page but the first and last; Skip on Find the box, Pick a character and Wi-Fi.
3. On Connect agents the forward button reads "Not now" until at least one agent is connected, then "Next".
4. While a firmware flash runs, Skip and Next on Find the box are disabled, so nobody pages away from the progress.
5. Done: "Get started" sets `onboardingDone` and closes the window. Closing the window early leaves it unset, so the guide returns at the next launch.

## Final state (output)

- The guide is finished; later launches show no window (LIFE-02).

## Check on Linux

- [ ] Known difference: the Linux guide is Welcome, Find the box, Pick a character, Wi-Fi, Done. Same Skip and Back rules, and no paging away during a flash?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
