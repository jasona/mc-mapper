package net.minecraft.client.multiplayer;
public class ServerData {
    public String name;
    private final boolean realm;
    public ServerData(String name, boolean realm) { this.name = name; this.realm = realm; }
    public boolean isRealm() { return realm; }
}
