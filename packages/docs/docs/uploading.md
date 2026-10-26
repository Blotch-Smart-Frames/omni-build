---
title: Uploading to the stores
description: Upload an Android app bundle to a Google Play track with omni upload, as a Google Cloud service account, without the Play Console's upload page or Gradle.
---

# Uploading to the stores

:::warning[Phase 3]

Google Play, for now: app bundles (AAB), released to a track. OmniBuild's requests follow Google's published description of its API, but no real Play Console app has taken an upload from it yet. Install OmniBuild from npm, as [Installation](./installation.md) shows.

:::

```sh
omni upload app.aab --service-account service-account.json                # to internal testing
omni upload app.aab --service-account service-account.json --track beta   # to open testing
omni upload app.aab --service-account service-account.json --draft        # a draft, to roll out in the Play Console
```

`upload` sends an app bundle to Google Play through the Google Play Developer API, and releases it to a track, as creating a release in the Play Console does. Google Play then makes the APKs each device installs from the bundle, and signs them with the app's signing key, which it keeps (Play App Signing).

## Google Play

`upload` reads the app's ID from the bundle, then makes one edit of the app, which Google Play publishes all at once:

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
