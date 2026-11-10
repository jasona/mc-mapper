package mcmap.oracle;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.FileOutputStream;
import java.io.PrintStream;
import java.lang.reflect.Proxy;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.Optional;
import java.util.function.Supplier;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.Registry;
import net.minecraft.core.registries.Registries;
import net.minecraft.world.level.LevelHeightAccessor;
import net.minecraft.world.level.WorldGenLevel;
import net.minecraft.world.level.biome.BiomeManager;
import net.minecraft.world.level.biome.FeatureSorter;
import net.minecraft.world.level.biome.MultiNoiseBiomeSource;
import net.minecraft.world.level.biome.MultiNoiseBiomeSourceParameterLists;
import net.minecraft.world.level.chunk.ChunkGenerator;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator;
import net.minecraft.world.level.levelgen.NoiseGeneratorSettings;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.WorldgenRandom;
import net.minecraft.world.level.levelgen.XoroshiroRandomSource;
import net.minecraft.world.level.levelgen.placement.PlacedFeature;
import net.minecraft.world.level.levelgen.placement.PlacementContext;
import net.minecraft.world.level.levelgen.placement.PlacementModifier;

/**
 * Decoration features, the game's way.
 *
 *   featorder <overworld|nether|end> <out.json>
 *     The generator's own per-step feature order ({"steps":[[placed feature id, ...], ...]}),
 *     which sets each feature's random seed. `extract` stores it in the pack.
 *
 *   featcands <seed> <dim> <cx0> <cz0> <cx1> <cz1> <out.ndjson> <placed feature id,...>
 *     Where each feature's placement modifiers put it, chunk by chunk, with the decoration seed
 *     ChunkGenerator.applyBiomeDecoration gives it. Biomes come from the BiomeManager (the
 *     zoomed lookup) and heightmaps from the generator's noise terrain. Modifiers that read
 *     placed blocks (block predicates, environment scans) can't run without a world: the
 *     position is written as it stands before the first of them, with "unchecked":true.
 *     Whether the feature itself then succeeds is not modelled.
 */
final class Feats {
    static final class World {
        final HolderLookup.Provider reg;
        final NoiseBasedChunkGenerator gen;
        final RandomState random;
        final BiomeManager biomes;
        final LevelHeightAccessor height;
        final long seed;

        World(HolderLookup.Provider reg, long seed, String dim) {
            this.reg = reg;
            this.seed = seed;
            boolean over = dim.equals("overworld");
            Holder<NoiseGeneratorSettings> settings = reg.lookupOrThrow(Registries.NOISE_SETTINGS).getOrThrow(
                over ? NoiseGeneratorSettings.OVERWORLD : dim.equals("nether") ? NoiseGeneratorSettings.NETHER : NoiseGeneratorSettings.END);
            net.minecraft.world.level.biome.BiomeSource source = dim.equals("end")
                ? net.minecraft.world.level.biome.TheEndBiomeSource.create(reg.lookupOrThrow(Registries.BIOME))
                : MultiNoiseBiomeSource.createFromPreset(reg.lookupOrThrow(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST)
                    .getOrThrow(over ? MultiNoiseBiomeSourceParameterLists.OVERWORLD : MultiNoiseBiomeSourceParameterLists.NETHER));
            gen = new NoiseBasedChunkGenerator(source, settings);
            random = RandomState.create(reg.lookupOrThrow(Registries.NOISE), seed, settings.value());
            var sampler = random.createClimateSampler(net.minecraft.world.level.levelgen.densityfunction.SamplerContext.builder().enableCaches().build());
            biomes = new BiomeManager(source.createResolver(sampler), BiomeManager.obfuscateSeed(seed));
            height = over ? LevelHeightAccessor.create(-64, 384) : LevelHeightAccessor.create(0, 256);
        }

        @SuppressWarnings("unchecked")
        List<FeatureSorter.StepFeatureData> steps() throws Exception {
            var f = ChunkGenerator.class.getDeclaredField("featuresPerStep");
            f.setAccessible(true);
            return ((Supplier<List<FeatureSorter.StepFeatureData>>) f.get(gen)).get();
        }

        /** A WorldGenLevel that answers only what placement modifiers ask. */
        WorldGenLevel level() {
            return (WorldGenLevel) Proxy.newProxyInstance(Feats.class.getClassLoader(), new Class<?>[] { WorldGenLevel.class }, (p, m, args) -> {
                String n = m.getName();
                int k = args == null ? 0 : args.length;
                switch (n) {
                    case "getMinY": return height.getMinY();
                    case "getHeight":
                        if (k == 0) return height.getHeight();
                        if (k == 3) return gen.getBaseHeight((Integer) args[1], (Integer) args[2], (Heightmap.Types) args[0], height, random);
                        break;
                    case "getMaxY": return height.getMaxY();
                    case "getSeed": return seed;
                    case "getBiome": return biomes.getBiome((BlockPos) args[0]);
                    case "toString": return "oracle level";
                    case "hashCode": return 0;
                    case "equals": return p == args[0];
                    default: break;
                }
                throw new Blocked(n);
            });
        }
    }

    /** A modifier asked the level for something it can't answer without placed blocks. */
    static final class Blocked extends RuntimeException {
        Blocked(String what) { super(what, null, false, false); }
    }

    static String id(Registry<PlacedFeature> r, PlacedFeature f) {
        return r.getResourceKey(f).map(k -> k.identifier().toString()).orElse("?");
    }

    static void order(HolderLookup.Provider reg, String[] a) throws Exception {
        World w = new World(Structs.loadRegistries(), 0, a[1]);
        Registry<PlacedFeature> r = ((net.minecraft.core.RegistryAccess) w.reg).lookupOrThrow(Registries.PLACED_FEATURE);
        JsonArray steps = new JsonArray();
        for (FeatureSorter.StepFeatureData s : w.steps()) {
            JsonArray l = new JsonArray();
            for (PlacedFeature f : s.features()) l.add(id(r, f));
            steps.add(l);
        }
        JsonObject o = new JsonObject();
        o.add("steps", steps);
        try (PrintStream out = new PrintStream(new FileOutputStream(a[2]), false, StandardCharsets.UTF_8)) {
            out.println(o);
        }
    }

    static void cands(HolderLookup.Provider ignored, PrintStream log, String[] a) throws Exception {
        long seed = Long.parseLong(a[1]);
        World w = new World(Structs.loadRegistries(), seed, a[2]);
        int cx0 = Integer.parseInt(a[3]), cz0 = Integer.parseInt(a[4]), cx1 = Integer.parseInt(a[5]), cz1 = Integer.parseInt(a[6]);
        Registry<PlacedFeature> r = ((net.minecraft.core.RegistryAccess) w.reg).lookupOrThrow(Registries.PLACED_FEATURE);
        List<FeatureSorter.StepFeatureData> steps = w.steps();
        record Target(String id, PlacedFeature f, int step, int index) {}
        List<Target> targets = new ArrayList<>();
        for (String want : a[8].split(",")) {
            PlacedFeature f = r.getValue(net.minecraft.resources.Identifier.parse(want));
            if (f == null) throw new IllegalArgumentException("no placed feature " + want);
            boolean found = false;
            for (int s = 0; s < steps.size(); s++) {
                int i = steps.get(s).indexMapping().applyAsInt(f);
                if (steps.get(s).features().contains(f)) {
                    targets.add(new Target(want, f, s, i));
                    found = true;
                }
            }
            if (!found) log.println("  " + want + " is in no step of this dimension");
        }
        WorldGenLevel level = w.level();
        int lines = 0;
        try (PrintStream out = new PrintStream(new FileOutputStream(a[7]), false, StandardCharsets.UTF_8)) {
            JsonObject meta = new JsonObject();
            meta.addProperty("meta", true); meta.addProperty("seed", seed); meta.addProperty("dimension", a[2]);
            meta.addProperty("cx0", cx0); meta.addProperty("cz0", cz0); meta.addProperty("cx1", cx1); meta.addProperty("cz1", cz1);
            JsonArray ts = new JsonArray();
            for (Target t : targets) {
                JsonObject j = new JsonObject();
                j.addProperty("id", t.id); j.addProperty("step", t.step); j.addProperty("index", t.index);
                ts.add(j);
            }
            meta.add("features", ts);
            out.println(meta);
            for (int cz = cz0; cz <= cz1; cz++) {
                for (int cx = cx0; cx <= cx1; cx++) {
                    BlockPos origin = new BlockPos(cx * 16, w.height.getMinY(), cz * 16);
                    WorldgenRandom rnd = new WorldgenRandom(new XoroshiroRandomSource(0L));
                    long deco = rnd.setDecorationSeed(seed, origin.getX(), origin.getZ());
                    for (Target t : targets) {
                        rnd.setFeatureSeed(deco, t.index, t.step);
                        PlacementContext ctx = new PlacementContext(level, w.gen, Optional.of(t.f));
                        List<Object[]> found = new ArrayList<>();
                        walk(t.f.placement(), 0, ctx, rnd, origin, null, found);
                        for (Object[] f : found) {
                            BlockPos p = (BlockPos) f[0];
                            JsonObject j = new JsonObject();
                            j.addProperty("f", t.id); j.addProperty("cx", cx); j.addProperty("cz", cz);
                            j.addProperty("x", p.getX()); j.addProperty("y", p.getY()); j.addProperty("z", p.getZ());
                            if (f[1] != null) j.addProperty("unchecked", (String) f[1]);
                            out.println(j); lines++;
                        }
                    }
                }
            }
        }
        log.println("wrote " + lines + " feature positions to " + a[7]);
    }

    /** The modifiers depth first, as PlacedFeature.place runs them (the feature itself is not
     *  placed). A modifier that reads placed blocks is skipped and named on the result. */
    static void walk(List<PlacementModifier> mods, int i, PlacementContext ctx, WorldgenRandom rnd, BlockPos pos, String skipped, List<Object[]> out) {
        if (i == mods.size()) {
            out.add(new Object[] { pos, skipped });
            return;
        }
        PlacementModifier m = mods.get(i);
        String type = m.getClass().getSimpleName();
        if (type.equals("BlockPredicateFilter") || type.equals("EnvironmentScanPlacement")) {
            walk(mods, i + 1, ctx, rnd, pos, type, out);
            return;
        }
        m.modify(ctx, rnd, pos, p -> walk(mods, i + 1, ctx, rnd, p, skipped, out));
    }
}
