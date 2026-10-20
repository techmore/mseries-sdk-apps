/* Original MSeries read-only IMCore adapter. Run with macOS osascript -l JavaScript.
 * Makes no Apple Events calls, signing overrides or SIP changes.
 * Private interfaces are probed at runtime; unavailable is never an empty list.
 * Pin-store domain/key reference: beeper/platform-imessage (MIT), Defaults.swift
 * at 79383860ff40b5c146b209bd438b33e05b0a6d22. No upstream code is copied.
 */
ObjC.import('Foundation');

function fail(reason) { throw new Error(reason); }
function selector(name) { return $.NSSelectorFromString(name); }
// Objective-C nil is a truthy JXA wrapper. Use the bridge's pointer check,
// otherwise a denied/missing NSData can be passed to the plist parser as nil.
function hasObject(value) { return !!value && !value.isNil(); }
function signature(object, name) {
    if (!hasObject(object) || !object.respondsToSelector(selector(name))) return null;
    var sig = object.methodSignatureForSelector(selector(name));
    if (!hasObject(sig) || Number(sig.numberOfArguments) !== 2) return null;
    return ObjC.unwrap($.NSString.stringWithUTF8String(sig.methodReturnType));
}
function objectValue(object, name, required) {
    var type = signature(object, name);
    if (!type || type[0] !== '@') {
        if (required) fail('Unsupported object accessor: ' + name);
        return null;
    }
    return object.performSelector(selector(name));
}
function plain(value) {
    return hasObject(value) ? ObjC.deepUnwrap(value) : null;
}
function readConfiguration(configuration, source) {
    var config = plain(configuration);
    if (!config || !Array.isArray(config.pP) || (config.pV !== 1 && config.pV !== 2))
        fail('Apple has not supplied a supported pin configuration');
    var stamp = configuration.objectForKey('pT');
    if (!hasObject(stamp) || !stamp.isKindOfClass($.NSDate))
        fail('Apple pin configuration is missing its modification time');
    var modified = Number(stamp.timeIntervalSince1970);
    if (!isFinite(modified) || modified * 1000 > Date.now() + 300000)
        fail('Apple pin configuration has an invalid modification time');
    var snapshot = makeSnapshot(config.pP, config.pZ || {});
    snapshot.source = source;
    snapshot.source_modified_at = new Date(modified * 1000).toISOString();
    snapshot.configuration_version = config.pV;
    return snapshot;
}
function readController(controller) {
    // Read the local configuration directly. Do not call extra refresh,
    // synchronization or migration helpers: those can have write side effects.
    // A process-support flag does not itself prove this getter is inaccessible.
    var configuration = objectValue(controller, '_locallyStoredPinConfiguration', true);
    if (!hasObject(configuration)) fail('No local IMCore pin configuration was returned');
    return readConfiguration(configuration, 'macos-imcore');
}
function readPinCache(path, diagnostics) {
    // NSError** uses a bridged nil object, as documented by Apple's JXA
    // examples. Ref() is for explicit C pointers and can crash on this path.
    var error = $();
    var data = $.NSData.dataWithContentsOfFileOptionsError(path, 0, error);
    if (!hasObject(data)) {
        var code = hasObject(error) ? Number(error.code) : 0;
        diagnostics.cache_error_code = code;
        diagnostics.cache_error_domain = hasObject(error) ? ObjC.unwrap(error.domain) : 'unknown';
        diagnostics.cache = (code === 257 || code === 13 || code === 1) ? 'access-denied' : code === 260 || code === 2 ? 'not-found' : 'read-error';
        return null;
    }
    if (Number(data.length) > 262144) fail('Apple pin cache is unexpectedly large');
    var plist = $.NSPropertyListSerialization.propertyListWithDataOptionsFormatError(data, 0, null, error);
    if (!hasObject(plist) || !plist.isKindOfClass($.NSDictionary)) fail('Apple pin cache is not a property list dictionary');
    var configuration = plist.objectForKey('pD');
    if (!hasObject(configuration)) {diagnostics.cache = 'not-initialized';return null;}
    var snapshot = readConfiguration(configuration, 'macos-pin-cache');
    diagnostics.cache = 'readable';
    return snapshot;
}
function makeSnapshot(identifiers, metadata) {
    if (!Array.isArray(identifiers) || identifiers.length > 32) fail('Unexpected pin list');
    var seen = Object.create(null);
    var pins = identifiers.map(function(identifier) {
        if (typeof identifier !== 'string' || !identifier.length || identifier.length > 2048 || /[\x00-\x1f\x7f]/.test(identifier) || seen[identifier])
            fail('Invalid or duplicate pin identifier');
        seen[identifier] = true;
        return {apple_identifier: identifier};
    });
    // Keep opaque group identity metadata for future qualified adapters. Never
    // read messages, chat.db, Apple Account credentials, or an entire KVS store.
    if (!metadata || typeof metadata !== 'object' || Array.isArray(metadata)) metadata = {};
    if (JSON.stringify(metadata).length > 65536) fail('Unexpected pin metadata size');
    return {schema: 'io.elementary.mseries.apple-message-pins', version: 1,
            source: 'macos-imcore', captured_at: new Date().toISOString(),
            source_os: ObjC.unwrap($.NSProcessInfo.processInfo.operatingSystemVersionString),
            pins: pins, metadata: metadata};
}
function saveSnapshot(snapshot, path) {
    if (path[0] !== '/') fail('Choose an absolute output path');
    var error = $();
    var text = $(JSON.stringify(snapshot, null, 2) + '\n');
    if (!text.writeToFileAtomicallyEncodingError(path, true, $.NSUTF8StringEncoding, error))
        fail('Could not save the private export');
    if (!$.NSFileManager.defaultManager.setAttributesOfItemAtPathError($({NSFilePosixPermissions: 384}), path, error))
        fail('Could not secure the private export permissions');
}
function selfTest() {
    console.log('Pin fixture: Objective-C adapter');
    ObjC.registerSubclass({name: 'MSeriesPinReaderFixture', methods: {
        fetchPinnedConversationIdentifiersFromLocalStore: {types: ['void', []], implementation: function() {}},
        _locallyStoredPinConfiguration: {types: ['id', []], implementation: function() {return $({pV: 2, pT: $.NSDate.date, pP: ['+15550100123', 'group-fixture']});}},
        chatMetadata: {types: ['id', []], implementation: function() {return $({});}}
    }});
    var fixture = $.MSeriesPinReaderFixture.alloc.init;
    if (signature(fixture, 'fetchPinnedConversationIdentifiersFromLocalStore') !== 'v') fail('Void ABI test failed');
    var snapshot = readController(fixture);
    if (snapshot.pins.length !== 2 || snapshot.pins[0].apple_identifier !== '+15550100123') fail('Ordered read test failed');
    if (makeSnapshot([], {}).pins.length !== 0) fail('Verified empty test failed');
    var rejected = false;
    try {makeSnapshot(['same', 'same'], {});} catch (_) {rejected = true;}
    if (!rejected) fail('Duplicate rejection failed');
    ObjC.registerSubclass({name: 'MSeriesUninitializedPinFixture', methods: {
        pinnedConversationIdentifiers: {types: ['id', []], implementation: function() {return $([]);}},
        hasCompletedInitialization: {types: ['bool', []], implementation: function() {return false;}}
    }});
    rejected = false;
    try {readController($.MSeriesUninitializedPinFixture.alloc.init);} catch (_) {rejected = true;}
    if (!rejected) fail('Uninitialized empty configuration was accepted');
    console.log('Pin fixture: binary cache');
    // Exercise the actual Foundation binary-plist reader against a private
    // temporary fixture, never the runner's or owner's Messages preferences.
    var path = ObjC.unwrap($.NSTemporaryDirectory()) + 'mseries-pin-fixture-' + ObjC.unwrap($.NSUUID.UUID.UUIDString) + '.plist';
    var error = $();
    try {
        var plist = $({pD: {pV: 2, pT: $.NSDate.date, pP: ['+15550100123', 'group-fixture']}});
        var data = $.NSPropertyListSerialization.dataWithPropertyListFormatOptionsError(plist, $.NSPropertyListBinaryFormat_v1_0, 0, error);
        if (!hasObject(data) || !data.writeToFileAtomically(path, true)) fail('Could not create the private cache fixture');
        var diagnostic = {};
        var cached = readPinCache(path, diagnostic);
        if (!cached || cached.source !== 'macos-pin-cache' || cached.pins[1].apple_identifier !== 'group-fixture' || diagnostic.cache !== 'readable') fail('Binary cache read test failed');
        var output = path + '.json';
        try {
            saveSnapshot(cached, output);
            var exported = JSON.parse(ObjC.unwrap($.NSString.stringWithContentsOfFileEncodingError(output, $.NSUTF8StringEncoding, error)));
            var attributes = $.NSFileManager.defaultManager.attributesOfItemAtPathError(output, error);
            if (exported.pins.length !== 2 || Number(ObjC.unwrap(attributes.objectForKey('NSFilePosixPermissions'))) !== 384) fail('Private JSON export round trip failed');
        } finally {$.NSFileManager.defaultManager.removeItemAtPathError(output, error);}
        console.log('Pin fixture: denied cache');
        if (ObjC.unwrap($.NSUserName()) !== 'root') {
            if (!$.NSFileManager.defaultManager.setAttributesOfItemAtPathError($({NSFilePosixPermissions: 0}), path, error)) fail('Could not prepare denied-read fixture');
            try {
                var denied = {};
                if (readPinCache(path, denied) !== null || denied.cache !== 'access-denied') fail('Permission denial classification failed');
            } finally {$.NSFileManager.defaultManager.setAttributesOfItemAtPathError($({NSFilePosixPermissions: 384}), path, error);}
        }
        console.log('Pin fixture: empty cache');
        var empty = $({pD: {pV: 2, pT: $.NSDate.date, pP: []}});
        var emptyData = $.NSPropertyListSerialization.dataWithPropertyListFormatOptionsError(empty, $.NSPropertyListBinaryFormat_v1_0, 0, error);
        if (!emptyData.writeToFileAtomically(path, true) || readPinCache(path, {}).pins.length !== 0) fail('Initialized empty cache test failed');
        console.log('Pin fixture: missing cache');
        var missing = {};
        if (readPinCache(path + '.missing', missing) !== null || missing.cache !== 'not-found') fail('Missing cache classification failed');
    } finally {$.NSFileManager.defaultManager.removeItemAtPathError(path, error);}
    return 'Pin reader fixture tests passed; no Apple account accessed.';
}
function run(args) {
    if (args.length === 1 && args[0] === '--self-test') return selfTest();
    var probe = args.length === 1 && args[0] === '--probe';
    if (!probe && (args.length !== 2 || args[0] !== '--output'))
        fail('Usage: Export-Message-Pins.js --output /absolute/private/file.json | --probe');
    var diagnostics = {};
    var snapshot = null;
    try {
      try {
        var bundle = $.NSBundle.bundleWithPath('/System/Library/PrivateFrameworks/IMCore.framework');
        if (!hasObject(bundle) || !bundle.load) fail('IMCore could not be loaded');
        var cls = $.NSClassFromString('IMPinnedConversationsController');
        if (!hasObject(cls)) fail('Messages pin controller is unavailable on this macOS version');
        var controller = objectValue(cls, 'sharedInstance', true);
        if (!hasObject(controller)) fail('Messages did not provide a pin controller');
        diagnostics.fetch_return_type = signature(controller, 'fetchPinnedConversationIdentifiersFromLocalStore');
        diagnostics.local_configuration_return_type = signature(controller, '_locallyStoredPinConfiguration');
        if (signature(cls, 'processSupportsPinnedConversations'))
            diagnostics.process_supported = Boolean(cls.processSupportsPinnedConversations);
        if (signature(cls, 'currentPinConfigurationVersion'))
            diagnostics.configuration_version = Number(cls.currentPinConfigurationVersion);
        try {snapshot = readController(controller);}
        catch (_) {diagnostics.framework_configuration = 'unavailable';}
      } catch (_) {diagnostics.framework = 'unavailable';}
        if (!snapshot) {
            var path = ObjC.unwrap($.NSHomeDirectory()) + '/Library/Preferences/com.apple.messages.pinning.plist';
            snapshot = readPinCache(path, diagnostics);
        }
        if (!snapshot) {
            if (diagnostics.cache === 'access-denied')
                fail('macOS denied access to the pin cache. Grant the launching Terminal app Full Disk Access in System Settings, reopen Terminal, and retry. No export was created.');
            fail('No initialized Apple pin cache is available. Open Messages and confirm its pins match the iPhone before exporting. This no-account probe cannot verify a signed-in export.');
        }
        if (probe) return JSON.stringify({status: 'ready', source: snapshot.source, count: snapshot.pins.length, diagnostics: diagnostics, content_printed: false});
        var path = args[1];
        saveSnapshot(snapshot, path);
        return 'Exported ' + snapshot.pins.length + ' ordered Messages pins. This is a snapshot; no phone settings changed.';
    } catch (error) {
        if (probe) return JSON.stringify({status: 'unavailable', reason: String(error.message || error), diagnostics: diagnostics, content_printed: false});
        throw error;
    }
}
