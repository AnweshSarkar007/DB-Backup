# Google Drive setup (Phase 7)

Google Drive backup uses your own OAuth **Desktop app** client so the app never ships shared
credentials. Setup is a one-time step per machine/organization.

## 1. Create an OAuth client in Google Cloud

1. Go to the [Google Cloud Console](https://console.cloud.google.com/), create (or pick) a project.
2. **APIs & Services → Enable APIs → Google Drive API** → enable.
3. **APIs & Services → OAuth consent screen**: configure it (External or Internal), and add your
   Google account under **Test users** (if the app stays in "Testing").
4. **APIs & Services → Credentials → Create credentials → OAuth client ID → Application type:
   Desktop app**. Copy the **Client ID** and **Client secret**.

The app requests only the `https://www.googleapis.com/auth/drive.file` scope — it can see and manage
**only the files it creates**, never the rest of your Drive.

## 2. Configure the app

- Destinations page → **Google Drive** → paste the Client ID and Client secret → **Save Google
  client**. The client id is stored in the local database; the client secret goes into Windows
  Credential Manager.

## 3. Connect an account

- Click **Connect Google Drive**. A browser window opens on a `127.0.0.1` loopback address; approve
  access and return to the app. The **refresh token** is stored in Windows Credential Manager (never
  in SQLite, never in logs). A folder named `Database Backup Manager` is created in your Drive.

You can connect **multiple accounts** — repeat the Connect step; each becomes its own destination.

## How backups use it

A verified dump is uploaded (resumably, so large files stream without loading into memory) into the
account's `Database Backup Manager` folder, then verified by comparing the remote size. Add the
Google Drive destination to a schedule (or select it in "Backup Now") to fan out to it alongside
local disks.

## Security notes

- Refresh tokens and the client secret live in Windows Credential Manager; the database stores only
  references (`gdrive:<id>`, `google:client_secret`).
- Access tokens are obtained on demand from the refresh token and never persisted.
- Revoking access in your Google account, or deleting the destination, stops future uploads.
