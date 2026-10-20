"""Presentation decisions shared by the native phone wizard and its tests."""
from dataclasses import dataclass


@dataclass(frozen=True)
class Outcome:
    state: str
    title: str
    detail: str


def connection_outcome(status, elapsed=0):
    if status.get('map') is True:
        return Outcome('ready', 'Your messages are connected', 'Open Messages to read and reply. Photos, reactions and older history are not available through this Bluetooth connection.')
    if status.get('connectivity_state') == 'authorization-required':
        return Outcome('permissions', 'Allow messages on your iPhone', 'On your iPhone, open Settings → Bluetooth → ⓘ beside this computer’s name. Enable Receive Messages (or Show Message Notifications) and Sync Contacts. This page will continue automatically when messages connect.')
    if elapsed >= 45:
        return Outcome('blocked', 'The phone has not allowed messages yet', 'Pairing is complete, but message access is unavailable. We will keep checking. Open connection details for the next recovery step; repeating pairing will not necessarily help.')
    return Outcome('checking', 'Checking the phone connection', 'Keep your iPhone unlocked and nearby while we verify message access.')


def needs_ios27_audio_recovery(ios_version, status, attempted):
    return ios_version.strip().startswith('27.') and status.get('connectivity_state') == 'authorization-required' and not attempted
