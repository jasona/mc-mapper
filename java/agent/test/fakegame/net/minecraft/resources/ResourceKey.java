package net.minecraft.resources;
public class ResourceKey {
    private final String id;
    public ResourceKey(String id) { this.id = id; }
    public Object identifier() { return id; }
}
