---
title: Uploading to the stores
description: Upload an Android app bundle to a Google Play track, or an iOS app to App Store Connect for TestFlight, with omni upload, without the Play Console's upload page, Xcode or Transporter.
---

# Uploading to the stores

:::warning[Phase 3]

Android app bundles (AAB), released to a Google Play track, and iOS apps built for devices (an `.ipa`), for TestFlight. OmniBuild's requests follow Google's and Apple's published descriptions of their APIs, but neither store has taken a real app from it yet. Install OmniBuild from npm, as [Installation](./installation.md) shows.

:::

```sh
omni upload app.aab --service-account service-account.json                # to internal testing
omni upload app.aab --service-account service-account.json --track beta   # to open testing
omni upload app.aab --service-account service-account.json --draft        # a draft, to roll out in the Play Console
omni upload app.ipa --api-key AuthKey_2X9R4HXF34.p8 --api-issuer 57246542-96fe-1a63-e053-0824d011072a  # for TestFlight
```

`upload` sends an app to its store through the store's own API: an app bundle to Google Play, through the Google Play Developer API, and an iOS app to App Store Connect, through the App Store Connect API, with no Play Console upload page, Xcode or Transporter.

## Google Play

`upload` releases an app bundle to a track, as creating a release in the Play Console does. Google Play then makes the APKs each device installs from the bundle, and signs them with the app's signing key, which it keeps (Play App Signing).

It reads the app's ID from the bundle, then makes one edit of the app, which Google Play publishes all at once:

- It uploads the bundle, which Google Play checks: that it's signed with the app's upload key, and that its version code is new.
- It releases the bundle to a track: `internal`, unless `--track` names another, `alpha` for closed testing, `beta` for open testing, `production`, or a closed testing track of the app's own, by its name. The release is rolled out to the track's testers, or with `--draft`, left a draft to roll out from the Play Console.
- It commits the edit. Until then, nothing changes, so a failure on the way leaves the app as it was.

### Before the first upload

Google Play's API releases apps the Play Console has already, so a new app starts there:

1. **Create the app** in the Play Console, under **All apps**.
2. **Upload its first app bundle by hand**, under **Internal testing**: that gives the app its ID, by which the API knows it. Sign it with your upload key, as `repackage --signing-key` does with a key from `omni key create`. Google Play takes only bundles signed with the same key from then on.
3. **Create a service account** in Google Cloud, in a project of yours, under **IAM & Admin › Service accounts**, turn on the **Google Play Android Developer API** in the same project, and create a JSON key for the account, under its **Keys**. That file is `--service-account`.
4. **Invite the service account** in the Play Console, under **Users and permissions**, by its email address, with permission to release the app to testing tracks, and to production if it will.

Until Google Play has published the app, it takes only draft releases of it: give `--draft`, and roll the release out from the Play Console.

### The service account's key

The key is a JSON file with the account's email address and an RSA private key. OmniBuild signs a token with it, which Google exchanges for an access token to Google Play's API, as Google's own libraries do, so the key itself never leaves the computer. In CI, write it to a file from a secret. OmniBuild reaches Google over HTTPS, through a proxy if `HTTPS_PROXY` names one.

### When an upload fails

OmniBuild gives Google Play's reason. The ones you'll meet first:

- **The bundle is signed with the wrong key**: another than the app's upload key, such as the debug key OmniBuild signs with when `repackage` gets no `--signing-key`. Repackage it with `--signing-key` and your upload key.
- **The version code has already been used**: each upload needs a build number of its own. Give the app config a higher `build`.
- **Google Play has no app** by the bundle's ID that the service account can release: the app isn't in the Play Console yet, its first bundle hasn't been uploaded by hand, the bundle has another ID, or the service account hasn't been invited to release it.
- **Only releases with status draft may be created on draft app**: Google Play hasn't published the app yet. Give `--draft`.

A bundle has to be an AAB: Google Play takes no APKs for new apps. To try an app on a device of your own, [install](./installing.md) its APK instead.

## App Store Connect

`upload` uploads an iOS app built for devices, an `.ipa`, to App Store Connect, as Xcode and Transporter upload one. It reads the app's bundle ID, version and build number from its `Info.plist`, then, through App Store Connect's API, with an API key:

- It finds the account's app with the bundle ID.
- It reserves room for the `.ipa`, for that version and build, and sends it to Apple's storage, in the parts App Store Connect asks for.
- It commits the upload, and waits while App Store Connect checks the app, which takes a few minutes. That's where App Store Connect refuses an app it won't take, and warns of what the app should fix before it's released.

Once App Store Connect has taken the app, Apple processes it for TestFlight, which takes a few minutes more, and TestFlight then offers it to the app's internal testers. TestFlight holds a build back until App Store Connect knows whether it uses encryption, so say in the app config, with `ios.usesNonExemptEncryption`, and each build does.

### Before the first upload

1. **Create the app** in App Store Connect, under **Apps**, with the app's bundle ID: the API can't. `omni provision` registers the bundle ID, or developer.apple.com does, under **Identifiers**.
2. **Sign the app for the App Store**, with an Apple Distribution certificate and App Store provisioning profiles, as `omni provision` gets them (its `--distribution` is `app-store` unless you say), and `repackage --signing-key --profile`. See [Signing](./repackaging.md#signing).
3. **Have an API key** with a role that may upload builds, such as App Manager, which the account's Admin creates in App Store Connect, under **Users and Access › Integrations**: the same kind of key `provision` takes, a `.p8` file, `AuthKey_<key ID>.p8`, with the team's issuer ID for a team key (`--api-issuer`). An individual key has none.

OmniBuild signs a token with the key for each request, as it does for `provision`, so the key itself never leaves the computer. In CI, write it to a file from a secret.

### When an upload fails

OmniBuild gives App Store Connect's reasons, each with its number, as Transporter does. The ones you'll meet first:

- **App Store Connect has no app** by the app's bundle ID: create it, or check the app config's `id`.
- **The build number has been used** (90189), or **the version is closed** for new builds (90062, 90186), as it is once a later version has been approved: give the app config a higher `build`, or a higher `version`.
- **A purpose string is missing** (90683): the shell uses an API that iOS asks its user to allow, without saying why. Give it in `ios.usageDescriptions`.
