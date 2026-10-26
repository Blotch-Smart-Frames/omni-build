---
title: Running from a build tool
description: How a framework or build tool runs OmniBuild, from Node with the @blotch/omni-build package's API or from anything else with --json, and reads what it reports.
---

# Running from a build tool

A framework's build command, or any other tool, runs OmniBuild as it would run a compiler: it passes a shell, an [app config](./repackaging.md#the-app-config) and keys as arguments, and gets apps back as files. What OmniBuild reports along the way, a key it created, a device waiting for its user, the result or the error, comes as events the tool can show in its own output.

## From Node

The `@blotch/omni-build` npm package runs the binary npm installed for the machine, and hands each event over, typed:

```ts
import { omniBuild } from '@blotch/omni-build';

const result = await omniBuild(
  ['repackage', 'shell.apk', '--config', 'app.json', '--out', 'app.apk'],
  { onEvent: (event) => console.log(event.message) },
);
// { event: 'repackaged', shell: 'shell.apk', out: 'app.apk', platform: 'android', format: 'apk', message: '...' }
```

`omniBuild()` takes the command's arguments as the `omni` command line does, and resolves with the last event, the command's result. When the command fails, it rejects with an `OmniBuildError` whose message is OmniBuild's reason. `onEvent` gets every event as it comes, the last included. Its other options are `cwd` and `env`, for the binary's process, and `binary`, a binary to run rather than npm's. `resolveBinary()` returns the path of npm's binary, for a tool that runs it itself.

## From anything else

With `--json`, OmniBuild writes each event as one line of JSON on stdout, and nothing else, on any command:

```sh
$ omni repackage shell.apk --out app.apk --json
{"event":"key-created","key":"debug","path":"/home/jane/.omni-build/android-debug-key.pem","message":"Created a debug key for signing Android apps: /home/jane/.omni-build/android-debug-key.pem"}
{"event":"repackaged","shell":"shell.apk","out":"app.apk","platform":"android","format":"apk","message":"Repackaged shell.apk into app.apk"}
```

Every event has an `event`, its name, and a `message`, the sentence OmniBuild shows people without `--json`. A tool that meets an event or a field it doesn't know, which newer versions of OmniBuild may add, ignores it, or shows the message.

| Event                  | When                                                                                         | Fields                                                                                                                                                                                    |
| ---------------------- | -------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `key-created`          | OmniBuild created a key: one it needed, or the upload key `key create` makes                 | `key`: `debug` (signs Android apps when given no key), `adb` (for [installing](./installing.md#the-adb-key)), `upload`, or `apple` (from `provision`); `path`                             |
| `key-requested`        | `key request` created a key for an Apple signing certificate, and its request                | `path`, the keystore, and `request`, the certificate signing request to upload                                                                                                            |
| `certificate-imported` | `key import` added a certificate to the keystore with its key                                | `path`, `certificate`, its name, and `expires`, a date                                                                                                                                    |
| `certificate-created`  | `provision` had Apple issue a certificate for the key in the keystore, and added it there    | `path`, `certificate`, its name, and `expires`, a date                                                                                                                                    |
| `app-id-registered`    | `provision` registered an App ID for one of the app's bundles                                | `bundleId`                                                                                                                                                                                |
| `capability-enabled`   | `provision` turned on a capability a bundle's entitlements need, for its App ID              | `bundleId`, and `capability`, as developer.apple.com names it, such as `Push Notifications`                                                                                               |
| `profile-created`      | `provision` had Apple make a provisioning profile for one of the app's bundles               | `bundleId`, and `name`, the profile's                                                                                                                                                     |
| `provisioned`          | `provision` wrote a provisioning profile for each of the app's bundles                       | `keystore`, `certificate` and `expires`, the certificate's, `distribution` (`app-store`, `ad-hoc` or `development`), `out`, the folder `--profile` takes, and `profiles`, each one's path |
| `authorizing`          | A device asks its user whether to allow this computer, and OmniBuild waits. Show the message | `device`, as `--device` names it                                                                                                                                                          |
| `repackaged`           | `repackage` wrote the app                                                                    | `shell`, `out`, `platform` (`android` or `ios`), `format` (`apk`, `aab`, `ipa`, or `app` for a zipped `.app`)                                                                             |
| `installed`            | `install` installed the app                                                                  | `app`, `device`, and `model`, the device's model, when it says                                                                                                                            |
| `listening`            | `tcpip` had the device listen for ADB on the network, where this computer reached it         | `device`, `model`, when it says, and `address`, which `--device` takes from then on                                                                                                       |
| `paired`               | `pair` paired this computer with the device for wireless debugging                           | `address`, where it listened for pairing, and `guid`, its name for itself, such as `adb-R5CT1234-a1B2c3`                                                                                  |
| `error`                | The command failed: the last event                                                           | Its message is the reason                                                                                                                                                                 |

Paths are as the arguments gave them, and a key OmniBuild creates in the home folder has its full path.

The exit code says how it ended. 0 is success, and the last event is the result. 1 is failure, and the last event is the `error`. 2 means OmniBuild didn't understand its arguments, which is a bug in the tool that ran it: it says why on stderr, as text, as it does without `--json`. Anything on stderr with `--json` is that, or a crash.
