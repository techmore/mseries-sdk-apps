action-close = Close
action-copy-magnet = Copy Magnet Link
action-open = Open Torrent File
action-open-description = Choose a .torrent file from your computer
action-magnet = Add Magnet Link
action-magnet-description = Paste a magnet link to start downloading
action-pause = Pause
action-prefs = Preferences
action-prefs-description = Set download folder and other preferences
action-quit = Quit
action-remove = Remove
action-resume = Resume
action-select-files = Select Files to Download
action-show-in-filemanager = Show in File Browser
advanced-preferences-title = Advanced
all-files-filter-description = All files
appmenu-tooltip = Application menu
filter-tooltip = Filter
filter-all = All
filter-downloading = Downloading
filter-seeding = Seeding
filter-paused = Paused
general-preferences-title = General
header-desktop-integration = Desktop Integration
header-security = Security
heading-download-location = Download Location
heading-limits = Limits
label-download-speed-limit = Download speed limit (KBps):
label-hide-on-close = Continue downloads when closed:
label-max-downloads = Max simultaneous downloads:
label-only-encrypted-peers = Only connect to encrypted peers:
label-port-number = Port number:
label-random-port = Randomise BitTorrent port on launch:
label-upload-speed-limit = Upload speed limit (KBps):
magnet-copied-notification = Magnet link copied
no-torrents-subtitle = Add a torrent file or a magnet link to begin downloading. You can also drop either one onto this window.
no-torrents-title = No Torrents
preferences-title = Preferences
state-checking = Checking
state-paused = Paused
state-waiting = Waiting in queue
tooltip-unlimited-hint = 0 means unlimited
torrent-files-filter-description = Torrent files

seconds = 
    { $seconds -> 
        [one] 1 second
        *[other] { $seconds } seconds
    }
minutes = 
    { $minutes -> 
        [one] 1 minute
        *[other] { $minutes } minutes
    }
hours = 
    { $hours -> 
        [one] 1 hour
        *[other] { $hours } hours
    }
days = 
    { $days -> 
        [one] 1 day
        *[other] { $days } days
    }

search-placeholder = Search torrents
open-torrent-tooltip = Open a .torrent file
open-magnet-tooltip = Add a magnet link
magnet-dialog-title = Add Magnet Link
magnet-dialog-label = Paste a magnet link:
magnet-dialog-add = Add
magnet-dialog-cancel = Cancel
magnet-added-notification = Torrent added
torrent-removed-notification = Torrent removed. Downloaded files were kept.
drop-unsupported-notification = Only .torrent files and magnet links can be added
download-complete-title = Download complete
current-download-folder = Downloads are saved to this folder
open-folder = Open Folder
no-matches-title = No Matching Torrents
no-matches-subtitle = Try a different search or filter.

status-downloading = { $done } of { $total } · { $percent }%
status-paused = Paused · { $done } of { $total } · { $percent }%
status-complete = Complete · { $total }
status-seeding = Seeding · { $total } · Uploaded { $uploaded } · Ratio { $ratio }
status-checking = Checking files · { $percent }%
status-stalled = Stalled · { $done } of { $total }
rate-line = ↓ { $down }/s   ↑ { $up }/s
rate-line-up = ↑ { $up }/s
remaining = { $time } left
time-seconds = { $seconds } sec
time-minutes = { $minutes } min
time-hours = { $hours } h { $minutes } min
time-days = { $days } d { $hours } h

status-count =
    { $count ->
        [one] 1 torrent
        *[other] { $count } torrents
    }
status-count-filtered = { $shown } of { $total } torrents
status-rates = ↓ { $down }/s   ↑ { $up }/s
