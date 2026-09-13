Endpoint: POST `/users/@me/settings-proto/${e}`


|  # | Proto field                | JS property             |
| -: | -------------------------- | ----------------------- |
|  1 | `versions`                 | `versions`              |
|  2 | `inbox`                    | `inbox`                 |
|  3 | `guilds`                   | `guilds`                |
|  4 | `user\_content`             | `userContent`           |
|  5 | `voice\_and\_video`          | `voiceAndVideo`         |
|  6 | `text\_and\_images`          | `textAndImages`         |
|  7 | `notifications`            | `notifications`         |
|  8 | `privacy`                  | `privacy`               |
|  9 | `debug`                    | `debug`                 |
| 10 | `game\_library`             | `gameLibrary`           |
| 11 | `status`                   | `status`                |
| 12 | `localization`             | `localization`          |
| 13 | `appearance`               | `appearance`            |
| 14 | `guild\_folders`            | `guildFolders`          |
| 15 | `favorites`                | `favorites`             |
| 16 | `audio\_context\_settings`   | `audioContextSettings`  |
| 17 | `communities`              | `communities`           |
| 18 | `broadcast`                | `broadcast`             |
| 19 | `clips`                    | `clips`                 |
| 20 | `for\_later`                | `forLater`              |
| 21 | `safety\_settings`          | `safetySettings`        |
| 22 | `icymi\_settings`           | `icymiSettings`         |
| 23 | `applications`             | `applications`          |
| 24 | `ads`                      | `ads`                   |
| 25 | `in\_app\_feedback\_settings` | `inAppFeedbackSettings` |
| 26 | `app\_version\_settings`     | `appVersionSettings`    |


/settings-proto/14 - GuildFolders

| Field | Name              | Type                   |
| ----: | ----------------- | ---------------------- | 
|   `1` | `folders`         | repeated `GuildFolder` |
|   `2` | `guild\_positions` | repeated `uint64`      |
