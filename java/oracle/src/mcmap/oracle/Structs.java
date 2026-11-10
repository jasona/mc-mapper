package mcmap.oracle;

import com.google.gson.JsonObject;
import java.io.FileOutputStream;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Optional;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.util.datafix.DataFixers;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.LevelHeightAccessor;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.MultiNoiseBiomeSource;
import net.minecraft.world.level.biome.MultiNoiseBiomeSourceParameterList;
import net.minecraft.world.level.biome.MultiNoiseBiomeSourceParameterLists;
import net.minecraft.world.level.chunk.ChunkGeneratorStructureState;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator;
import net.minecraft.world.level.levelgen.NoiseGeneratorSettings;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.WorldgenRandom;
import net.minecraft.world.level.levelgen.LegacyRandomSource;
import net.minecraft.world.level.levelgen.structure.Structure;
import net.minecraft.world.level.levelgen.structure.StructureSet;
import net.minecraft.world.level.levelgen.structure.StructureStart;
import net.minecraft.world.level.levelgen.structure.placement.ConcentricRingsStructurePlacement;
import net.minecraft.world.level.levelgen.structure.placement.StructurePlacement;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplateManager;
import net.minecraft.world.level.storage.LevelStorageSource;

/**
 * Ground truth for structures: the game's own placement, selection and Structure.generate, run
 * the way ChunkGenerator.createStructures runs them (no references, Overworld, default preset).
 *
 * Writes NDJSON to out, one line per chunk where a placement fires:
 *   {"set":..,"cx":..,"cz":..,"result":"<structure id>|none","x":..,"y":..,"z":..,"tried":[..]}
 * x/y/z is the generation stub position of the structure that generated. Strongholds also get one
 * line per ring position ("ring":true), and heights of the chunk centres are written for a few
 * heightmaps so the terrain approximation can be measured ("height" lines).
 */
final class Structs {
    static void run(HolderLookup.Provider reg, PrintStream log, String[] a) throws Exception {
        long seed = Long.parseLong(a[1]);
        int cx0 = Integer.parseInt(a[2]), cz0 = Integer.parseInt(a[3]);
        int cx1 = Integer.parseInt(a[4]), cz1 = Integer.parseInt(a[5]);
        // Load the worldgen registries from the vanilla data pack with tags bound, the way
        // WorldLoader does; the code-built lookup the biome commands use has no tags.
        Path tmp = Files.createTempDirectory("mcmap-oracle");
        LevelStorageSource.LevelStorageAccess storage = LevelStorageSource.createDefault(tmp).createAccess("w");
        MultiPackResourceManager rm = new MultiPackResourceManager(PackType.SERVER_DATA, List.of(ServerPacksSource.createVanillaPackSource().fullResources()));
        var layers = net.minecraft.server.RegistryLayer.createRegistryAccess();
        var pending = net.minecraft.tags.TagLoader.loadTagsForExistingRegistries(rm, layers.getLayer(net.minecraft.server.RegistryLayer.STATIC));
        var ctxLookups = net.minecraft.tags.TagLoader.buildUpdatedLookups(layers.getAccessForLoading(net.minecraft.server.RegistryLayer.WORLD), pending);
        RegistryAccess.Frozen worldgen = net.minecraft.resources.RegistryDataLoader.load(rm, ctxLookups, net.minecraft.resources.RegistryDataLoader.WORLD_REGISTRIES, Runnable::run).join();
        pending.forEach(net.minecraft.core.Registry.PendingTags::apply);
        layers = layers.replaceFrom(net.minecraft.server.RegistryLayer.WORLD, worldgen);
        RegistryAccess access = layers.compositeAccess();
        reg = access;
        Holder<NoiseGeneratorSettings> settings = reg.lookupOrThrow(Registries.NOISE_SETTINGS).getOrThrow(NoiseGeneratorSettings.OVERWORLD);
        Holder<MultiNoiseBiomeSourceParameterList> preset = reg.lookupOrThrow(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST)
            .getOrThrow(MultiNoiseBiomeSourceParameterLists.OVERWORLD);
        MultiNoiseBiomeSource source = MultiNoiseBiomeSource.createFromPreset(preset);
        NoiseBasedChunkGenerator gen = new NoiseBasedChunkGenerator(source, settings);
        RandomState random = RandomState.create(reg.lookupOrThrow(Registries.NOISE), seed, settings.value());
        HolderLookup<StructureSet> sets = reg.lookupOrThrow(Registries.STRUCTURE_SET);
        ChunkGeneratorStructureState state = gen.createState(sets, random, seed);
        StructureTemplateManager templates = new StructureTemplateManager(rm, storage, DataFixers.getDataFixer(), reg.lookupOrThrow(Registries.BLOCK));
        LevelHeightAccessor height = LevelHeightAccessor.create(-64, 384);
        net.minecraft.world.level.biome.Climate.Sampler sampler = random.createClimateSampler(
            net.minecraft.world.level.levelgen.densityfunction.SamplerContext.builder().enableCaches().build());
        int lines = 0;
        try (PrintStream o = new PrintStream(new FileOutputStream(a[6]), false, StandardCharsets.UTF_8)) {
            JsonObject meta = new JsonObject();
            meta.addProperty("meta", true); meta.addProperty("seed", seed);
            meta.addProperty("cx0", cx0); meta.addProperty("cz0", cz0); meta.addProperty("cx1", cx1); meta.addProperty("cz1", cz1);
            BlockPos spawn = net.minecraft.world.level.levelgen.NoiseSpawnFinder.findSpawnPosition(settings.value().spawnTarget(),
                random.samplersWithContext(net.minecraft.world.level.levelgen.densityfunction.SamplerContext.builder().enableCaches().build()));
            meta.addProperty("spawn_x", spawn.getX()); meta.addProperty("spawn_z", spawn.getZ());
            o.println(meta);
            for (Holder<StructureSet> hs : state.possibleStructureSets()) {
                String setId = hs.unwrapKey().map(k -> k.identifier().toString()).orElse("?");
                StructurePlacement placement = hs.value().placement();
                if (placement instanceof ConcentricRingsStructurePlacement rings) {
                    for (ChunkPos p : state.getRingPositionsFor(rings)) {
                        JsonObject j = new JsonObject();
                        j.addProperty("set", setId); j.addProperty("ring", true);
                        j.addProperty("cx", p.x()); j.addProperty("cz", p.z());
                        o.println(j); lines++;
                    }
                }
                for (int cz = cz0; cz <= cz1; cz++) {
                    for (int cx = cx0; cx <= cx1; cx++) {
                        if (!placement.isStructureChunk(state, cx, cz)) continue;
                        ChunkPos pos = new ChunkPos(cx, cz);
                        JsonObject j = new JsonObject();
                        j.addProperty("set", setId); j.addProperty("cx", cx); j.addProperty("cz", cz);
                        com.google.gson.JsonArray tried = new com.google.gson.JsonArray();
                        String result = "none";
                        List<StructureSet.StructureSelectionEntry> entries = new ArrayList<>(hs.value().structures());
                        if (entries.size() == 1) {
                            StructureStart s = attempt(entries.get(0), access, gen, source, random, templates, seed, pos, height, sampler, j, tried);
                            if (s != null) result = id(entries.get(0).structure());
                        } else {
                            WorldgenRandom r = new WorldgenRandom(new LegacyRandomSource(0L));
                            r.setLargeFeatureSeed(seed, cx, cz);
                            int total = 0;
                            for (StructureSet.StructureSelectionEntry e : entries) total += e.weight();
                            while (!entries.isEmpty()) {
                                int pick = r.nextInt(total);
                                int i = 0;
                                for (StructureSet.StructureSelectionEntry e : entries) {
                                    pick -= e.weight();
                                    if (pick < 0) break;
                                    i++;
                                }
                                StructureSet.StructureSelectionEntry e = entries.get(i);
                                if (attempt(e, access, gen, source, random, templates, seed, pos, height, sampler, j, tried) != null) {
                                    result = id(e.structure());
                                    break;
                                }
                                entries.remove(i);
                                total -= e.weight();
                            }
                        }
                        j.addProperty("result", result);
                        j.add("tried", tried);
                        BlockPos lp = placement.getLocatePos(pos);
                        j.addProperty("lx", lp.getX()); j.addProperty("lz", lp.getZ());
                        o.println(j); lines++;
                    }
                }
            }
            // Terrain heights at chunk centres, to measure the approximation the map uses.
            for (int cz = cz0; cz <= cz1; cz += 4) {
                for (int cx = cx0; cx <= cx1; cx += 4) {
                    int x = (cx << 4) + 8, z = (cz << 4) + 8;
                    JsonObject j = new JsonObject();
                    j.addProperty("height", true); j.addProperty("x", x); j.addProperty("z", z);
                    j.addProperty("ws", gen.getBaseHeight(x, z, Heightmap.Types.WORLD_SURFACE_WG, height, random));
                    j.addProperty("of", gen.getBaseHeight(x, z, Heightmap.Types.OCEAN_FLOOR_WG, height, random));
                    o.println(j); lines++;
                }
            }
        }
        log.println("wrote " + lines + " lines to " + a[6]);
    }

    private static String id(Holder<Structure> h) {
        return h.unwrapKey().map(k -> k.identifier().toString()).orElse("?");
    }

    /** One tryGenerateStructure: returns the start when it is valid, else null. Records the stub. */
    private static StructureStart attempt(StructureSet.StructureSelectionEntry e, RegistryAccess access, NoiseBasedChunkGenerator gen,
                                          MultiNoiseBiomeSource source, RandomState random, StructureTemplateManager templates,
                                          long seed, ChunkPos pos, LevelHeightAccessor height, net.minecraft.world.level.biome.Climate.Sampler sampler, JsonObject j, com.google.gson.JsonArray tried) {
        Structure s = e.structure().value();
        tried.add(id(e.structure()));
        java.util.function.Predicate<Holder<Biome>> valid = s.biomes()::contains;
        StructureStart start = s.generate(e.structure(), Level.OVERWORLD, access, gen, source, sampler, random, templates, seed, pos, 0, height, valid);
        if (!start.isValid()) return null;
        Structure.GenerationContext ctx = new Structure.GenerationContext(access, gen, source, sampler, random, templates, seed, pos, height, valid);
        Optional<Structure.GenerationStub> stub = s.findValidGenerationPoint(ctx);
        stub.ifPresent(st -> {
            j.addProperty("x", st.position().getX()); j.addProperty("y", st.position().getY()); j.addProperty("z", st.position().getZ());
        });
        var bb = start.getBoundingBox();
        j.addProperty("bb", bb.minX() + "," + bb.minY() + "," + bb.minZ() + "," + bb.maxX() + "," + bb.maxY() + "," + bb.maxZ());
        return start;
    }
}
