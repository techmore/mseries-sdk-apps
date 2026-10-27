private SignalNative.MessageEvent? parse (string json) {
    try {
        var parser = new Json.Parser ();
        parser.load_from_data (json);
        return SignalNative.MessageEvent.parse (parser.get_root (), "local");
    } catch (Error error) { assert_not_reached (); }
}
int main (string[] args) {
    Test.init (ref args);
    Test.add_func ("/event/private-number", () => {
        var message = parse ("""{"params":{"envelope":{"sourceNumber":null,"sourceUuid":"uuid","dataMessage":{"message":"Hello","timestamp":123}}}}""");
        assert (message != null && message.conversation_id == "uuid");
        assert (message.text == "Hello" && message.timestamp == 123 && !message.outgoing);
    });
    Test.add_func ("/event/group-sync", () => {
        var message = parse ("""{"params":{"result":{"account":"local","envelope":{"syncMessage":{"sentMessage":{"destinationNumber":null,"message":"Group","timestamp":456,"groupInfo":{"groupId":"group","groupName":"Friends"}}}}}}}""");
        assert (message != null && message.is_group && message.outgoing);
        assert (message.conversation_id == "group" && message.name == "Friends");
    });
    Test.add_func ("/event/unsupported", () => {
        assert (parse ("null") == null);
        assert (parse ("{}") == null);
        assert (parse ("""{"params":{"envelope":{"sourceNumber":"sender","dataMessage":{"message":null}}}}""") == null);
        assert (parse ("""{"params":{"envelope":{"sourceNumber":"sender","dataMessage":{"message":123}}}}""") == null);
        assert (parse ("""{"params":{"envelope":{"receiptMessage":{}}}}""") == null);
    });
    Test.add_func ("/event/account-isolation", () => {
        assert (parse ("""{"params":{"account":"other","envelope":{"sourceNumber":"sender","dataMessage":{"message":"Secret"}}}}""") == null);
    });
    Test.add_func ("/event/expiration", () => {
        var message = parse ("""{"params":{"envelope":{"sourceNumber":"sender","dataMessage":{"message":"Ephemeral","expiresInSeconds":60}}}}""");
        assert (message != null && !message.ephemeral && message.expiration_seconds == 60 && message.text == "Ephemeral");
    });
    return Test.run ();
}
