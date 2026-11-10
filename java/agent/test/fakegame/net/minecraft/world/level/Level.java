package net.minecraft.world.level;
public class Level {
    public net.minecraft.resources.ResourceKey dimension() { return new net.minecraft.resources.ResourceKey("minecraft:overworld"); }
    public net.minecraft.world.level.biome.BiomeManager getBiomeManager() { return new net.minecraft.world.level.biome.BiomeManager(); }
}
