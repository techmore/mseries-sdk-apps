"""Presentation decisions shared by the native phone wizard and its tests."""
from dataclasses import dataclass


@dataclass(frozen=True)
class Outcome:
    state: str
    title: str
    detail: str
    action: str | None = None


def connection_outcome(status, elapsed=0):
    if status.get('map') is True:
        return Outcome('ready', 'Your messages are connected', 'Open Messages to read and reply. Photos, reactions and older history are not available through this Bluetooth connection.', 'Open Messages')
    state = status.get('connectivity_state')
    if state == 'backend-inactive':
        return Outcome('inactive', 'Start your phone connection', 'Your saved pairing is retained. Retry Connection starts the phone service and checks message access.', 'Retry Connection')
    if state == 'status-unavailable':
        return Outcome('unavailable', 'Could not check message access', 'The phone service did not return its status. Refresh to check again.', 'Refresh Status')
    if status.get('bluetooth_connected') is False:
        return Outcome('blocked', 'Your iPhone is disconnected', 'Keep your iPhone nearby with Bluetooth enabled. Retry Connection uses the saved pairing.', 'Retry Connection')
    denied = state == 'authorization-required' or status.get('evidence', {}).get('messages') == 'access-denied'
    if denied and status.get('recovery_history'):
        return Outcome('blocked', 'Messages are still blocked', 'The last verified message request was denied by the iPhone. The previous recovery attempts did not fix it. Retry Connection checks the existing pairing; it does not forget your phone or change audio settings.', 'Retry Connection')
    if state == 'authorization-required':
        return Outcome('permissions', 'Allow messages on your iPhone', 'On your iPhone, open Settings → Bluetooth → ⓘ beside this computer’s name. Enable Receive Messages (or Show Message Notifications). This page will continue automatically when message access connects.')
    if elapsed >= 45:
        return Outcome('blocked', 'Messages have not connected', 'Message access is unavailable. Retry Connection checks the saved pairing; connection details retain the last verified result.', 'Retry Connection')
    return Outcome('checking', 'Checking message access', 'Keep your iPhone unlocked and nearby while we verify the connection.')


def needs_ios27_audio_recovery(ios_version, status, attempted):
    return ios_version.strip().startswith('27.') and status.get('connectivity_state') == 'authorization-required' and not attempted
