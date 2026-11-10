package mcmap.oracle;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.mojang.datafixers.util.Pair;
import java.io.BufferedOutputStream;
import java.io.DataOutputStream;
import java.io.FileOutputStream;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.SharedConstants;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.registries.Registries;
import net.minecraft.data.registries.VanillaRegistries;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.Climate;
import net.minecraft.world.level.biome.MultiNoiseBiomeSource;
import net.minecraft.world.level.biome.MultiNoiseBiomeSourceParameterList;
import net.minecraft.world.level.biome.MultiNoiseBiomeSourceParameterLists;
import net.minecraft.world.level.levelgen.NoiseGeneratorSettings;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.densityfunction.SamplerContext;

/**
 * Test oracle for MC Mapper: runs the game's own biome code from the client jar on the classpath.
 * Dev-only. Nothing here ships, and its output is never committed (it is Mojang's data).
 *
 * Commands use block coordinates. y defaults to 64; "surface" samples at y 64 and forces depth
 * to 0, the plan's shortcut for the surface biome:
 *   biome   <seed> <x> <z> [y]                      one biome id
 *   climate <seed> <x> <z> [y]                      the six quantized climate values and the biome
 *   grid    <seed> <x0> <z0> <w> <h> <step> <out>   biome ids for a w*h grid of cells, step blocks apart
 *   params  <out.json>                              the Overworld biome parameter table, in tree order
 *   climgrid <seed> <x0> <z0> <w> <h> <step> <out>  raw surface climate values and biomes, for diff tests
 *   dfgrid <seed> <function id> <x0> <z0> <w> <h> <step> <y> <out>  a density function's floats ("MCDF", w, h, then w*h f32)
 *   templates <out.json>                           start pool and ruined portal template geometry (see Templates)
 *   structs <seed> <cx0> <cz0> <cx1> <cz1> <out>     every Overworld structure attempt in a chunk range (see Structs)
 */
public final class Oracle {
    /** Pass "surface" as y to evaluate at depth 0 instead of a fixed height. */
    static final int SURFACE = Integer.MIN_VALUE;
    private static HolderLookup.Provider registries;
    private static PrintStream out;

    public static void main(String[] args) throws Exception {
        // The game logs through log4j to stdout during bootstrap; keep our answers on a clean stream.
        out = new PrintStream(new FileOutputStream(java.io.FileDescriptor.out), true, StandardCharsets.UTF_8);
        System.setOut(System.err);
        if (args.length == 0) { usage(); return; }
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        registries = VanillaRegistries.createWorldLookup();
        switch (args[0]) {
            case "biome" -> {
                World w = new World(Long.parseLong(args[1]));
                int y = args.length > 4 ? parseY(args[4]) : 64;
                out.println(w.biome(Integer.parseInt(args[2]), y, Integer.parseInt(args[3])));
            }
            case "climate" -> {
                World w = new World(Long.parseLong(args[1]));
                int x = Integer.parseInt(args[2]), z = Integer.parseInt(args[3]);
                int y = args.length > 4 ? parseY(args[4]) : 64;
                Climate.TargetPoint t = w.sampler.sample(x >> 2, (y == SURFACE ? 64 : y) >> 2, z >> 2);
                JsonObject o = new JsonObject();
                o.addProperty("temperature", t.temperature());
                o.addProperty("humidity", t.humidity());
                o.addProperty("continentalness", t.continentalness());
                o.addProperty("erosion", t.erosion());
                o.addProperty("depth", t.depth());
                o.addProperty("weirdness", t.weirdness());
                o.addProperty("biome", w.biome(x, y, z));
                out.println(o);
            }
            case "grid" -> grid(args);
            case "params" -> params(args[1], args.length > 2 ? args[2] : "overworld");
            case "dimgrid" -> dimgrid(args);
            case "climgrid" -> climgrid(args);
            case "structs" -> Structs.run(registries, out, args);
            case "featorder" -> Feats.order(registries, args);
            case "featcands" -> Feats.cands(registries, out, args);
            case "dfgrid" -> dfgrid(args);
            case "templates" -> Templates.run(args);
            default -> usage();
        }
        out.flush();
        System.exit(0);
    }

    private static int parseY(String s) {
        return s.equals("surface") ? SURFACE : Integer.parseInt(s);
    }

    private static void usage() {
        System.err.println("usage: biome <seed> <x> <z> [y] | climate <seed> <x> <z> [y] | grid <seed> <x0> <z0> <w> <h> <step> <out> [y] | params <out.json>");
    }

    /** One seed's Overworld biome source and climate sampler, built the way the server builds them. */
    static final class World {
        final Climate.Sampler sampler;
        final MultiNoiseBiomeSource source;

        World(long seed) {
            NoiseGeneratorSettings settings = registries.lookupOrThrow(Registries.NOISE_SETTINGS)
                .getOrThrow(NoiseGeneratorSettings.OVERWORLD).value();
            RandomState random = RandomState.create(registries.lookupOrThrow(Registries.NOISE), seed, settings);
            sampler = random.createClimateSampler(SamplerContext.EMPTY_UNCACHED);
            Holder<MultiNoiseBiomeSourceParameterList> preset = registries
                .lookupOrThrow(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST)
                .getOrThrow(MultiNoiseBiomeSourceParameterLists.OVERWORLD);
            source = MultiNoiseBiomeSource.createFromPreset(preset);
        }

        /** y == SURFACE evaluates the climate at y 64 and then forces depth to 0, the surface. */
        String biome(int x, int y, int z) {
            Climate.TargetPoint t = sampler.sample(x >> 2, (y == SURFACE ? 64 : y) >> 2, z >> 2);
            if (y == SURFACE) {
                t = new Climate.TargetPoint(t.temperature(), t.humidity(), t.continentalness(), t.erosion(), 0L, t.weirdness());
            }
            Holder<Biome> b = source.getNoiseBiome(t);
            return b.unwrapKey().map(k -> k.identifier().toString()).orElse("?");
        }
    }

    /**
     * Surface climate grid: magic "MCCG", u32 w, u32 h, u32 n names, the names as u16-length
     * UTF-8, then per cell (z outer, x inner) the five raw climate floats at the cell's quart
     * corner and y 64 (temperature, humidity, continentalness, erosion, weirdness) and the u16
     * surface biome. Biomes are looked up in that order, so the game's per-thread search hint
     * follows the same path a test can replay.
     */
    private static void climgrid(String[] a) throws Exception {
        World w = new World(Long.parseLong(a[1]));
        int x0 = Integer.parseInt(a[2]), z0 = Integer.parseInt(a[3]);
        int gw = Integer.parseInt(a[4]), gh = Integer.parseInt(a[5]), step = Integer.parseInt(a[6]);
        Map<String, Integer> index = new HashMap<>();
        List<String> names = new ArrayList<>();
        float[] vals = new float[gw * gh * 5];
        short[] cells = new short[gw * gh];
        Climate.Sampler s = w.sampler;
        for (int j = 0; j < gh; j++) {
            for (int i = 0; i < gw; i++) {
                int x = x0 + i * step, z = z0 + j * step;
                int bx = (x >> 2) << 2, bz = (z >> 2) << 2;
                int o = (j * gw + i) * 5;
                vals[o] = s.temperature().sampleValue(bx, 64, bz);
                vals[o + 1] = s.humidity().sampleValue(bx, 64, bz);
                vals[o + 2] = s.continentalness().sampleValue(bx, 64, bz);
                vals[o + 3] = s.erosion().sampleValue(bx, 64, bz);
                vals[o + 4] = s.weirdness().sampleValue(bx, 64, bz);
                String b = w.biome(x, SURFACE, z);
                Integer k = index.get(b);
                if (k == null) { k = names.size(); names.add(b); index.put(b, k); }
                cells[j * gw + i] = (short) (int) k;
            }
        }
        try (DataOutputStream o = new DataOutputStream(new BufferedOutputStream(new FileOutputStream(a[7])))) {
            o.writeBytes("MCCG");
            o.writeInt(gw);
            o.writeInt(gh);
            o.writeInt(names.size());
            for (String n : names) o.writeUTF(n);
            for (int c = 0; c < gw * gh; c++) {
                for (int k = 0; k < 5; k++) o.writeFloat(vals[c * 5 + k]);
                o.writeShort(cells[c]);
            }
        }
        out.println("wrote " + gw + "x" + gh + " climate cells, " + names.size() + " biomes, to " + a[7]);
    }

    /** Any registered density function, sampled uncached at block positions on a horizontal grid. */
    private static void dfgrid(String[] a) throws Exception {
        long seed = Long.parseLong(a[1]);
        NoiseGeneratorSettings settings = registries.lookupOrThrow(Registries.NOISE_SETTINGS).getOrThrow(NoiseGeneratorSettings.OVERWORLD).value();
        RandomState random = RandomState.create(registries.lookupOrThrow(Registries.NOISE), seed, settings);
        var df = registries.lookupOrThrow(Registries.DENSITY_FUNCTION)
            .getOrThrow(ResourceKey.create(Registries.DENSITY_FUNCTION, net.minecraft.resources.Identifier.parse(a[2]))).value();
        int x0 = Integer.parseInt(a[3]), z0 = Integer.parseInt(a[4]);
        int gw = Integer.parseInt(a[5]), gh = Integer.parseInt(a[6]), step = Integer.parseInt(a[7]), y = Integer.parseInt(a[8]);
        try (DataOutputStream o = new DataOutputStream(new BufferedOutputStream(new FileOutputStream(a[9])))) {
            o.writeBytes("MCDF");
            o.writeInt(gw); o.writeInt(gh);
            for (int j = 0; j < gh; j++)
                for (int i = 0; i < gw; i++)
                    o.writeFloat(random.sampleBlockValueUncached(df, x0 + i * step, y, z0 + j * step));
        }
        out.println("wrote " + gw + "x" + gh + " values of " + a[2] + " to " + a[9]);
    }

    /**
     * Grid of biomes as a small binary file: magic "MCBG", u32 w, u32 h, u32 n names, the names as
     * u16-length UTF-8, then w*h u16 indexes row by row (z outer, x inner).
     */
    private static void grid(String[] a) throws Exception {
        World w = new World(Long.parseLong(a[1]));
        int x0 = Integer.parseInt(a[2]), z0 = Integer.parseInt(a[3]);
        int gw = Integer.parseInt(a[4]), gh = Integer.parseInt(a[5]), step = Integer.parseInt(a[6]);
        int y = a.length > 8 ? parseY(a[8]) : 64;
        Map<String, Integer> index = new HashMap<>();
        List<String> names = new ArrayList<>();
        short[] cells = new short[gw * gh];
        for (int j = 0; j < gh; j++) {
            for (int i = 0; i < gw; i++) {
                String b = w.biome(x0 + i * step, y, z0 + j * step);
                Integer k = index.get(b);
                if (k == null) { k = names.size(); names.add(b); index.put(b, k); }
                cells[j * gw + i] = (short) (int) k;
            }
        }
        try (DataOutputStream o = new DataOutputStream(new BufferedOutputStream(new FileOutputStream(a[7])))) {
            o.writeBytes("MCBG");
            o.writeInt(gw); o.writeInt(gh); o.writeInt(names.size());
            for (String n : names) o.writeUTF(n);
            for (short c : cells) o.writeShort(c);
        }
        out.println("wrote " + gw + "x" + gh + " cells, " + names.size() + " biomes, to " + a[7]);
    }

    /** The Overworld parameter list, the table the jar builds in code rather than shipping as data. */
    private static void params(String path, String preset) throws Exception {
        MultiNoiseBiomeSourceParameterList list = registries
            .lookupOrThrow(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST)
            .getOrThrow(preset.equals("nether") ? MultiNoiseBiomeSourceParameterLists.NETHER : MultiNoiseBiomeSourceParameterLists.OVERWORLD).value();
        JsonArray arr = new JsonArray();
        for (Pair<Climate.ParameterPoint, Holder<Biome>> p : list.parameters().values()) {
            Climate.ParameterPoint pt = p.getFirst();
            JsonObject o = new JsonObject();
            o.addProperty("biome", p.getSecond().unwrapKey().map(ResourceKey::identifier).map(Object::toString).orElse("?"));
            o.add("temperature", range(pt.temperature()));
            o.add("humidity", range(pt.humidity()));
            o.add("continentalness", range(pt.continentalness()));
            o.add("erosion", range(pt.erosion()));
            o.add("depth", range(pt.depth()));
            o.add("weirdness", range(pt.weirdness()));
            o.addProperty("offset", pt.offset());
            arr.add(o);
        }
        JsonObject root = new JsonObject();
        root.addProperty("version", SharedConstants.getCurrentVersion().name());
        root.addProperty("preset", "minecraft:" + preset);
        root.addProperty("quantization", "Climate.quantizeCoord: value * 10000, as long");
        root.add("entries", arr);
        try (PrintStream f = new PrintStream(new FileOutputStream(path), false, StandardCharsets.UTF_8)) {
            f.println(root);
        }
        out.println("wrote " + arr.size() + " entries to " + path);
    }

    /**
     * dimgrid <seed> <nether|end> <x0> <z0> <w> <h> <step> <out.txt>: the dimension's biome at
     * each block (x0 + i * step, y 64, z0 + j * step), one id per line, rows of w.
     */
    private static void dimgrid(String[] a) throws Exception {
        long seed = Long.parseLong(a[1]);
        boolean nether = a[2].equals("nether");
        int x0 = Integer.parseInt(a[3]), z0 = Integer.parseInt(a[4]), w = Integer.parseInt(a[5]), h = Integer.parseInt(a[6]), step = Integer.parseInt(a[7]);
        if (a[2].equals("large_biomes") || a[2].equals("amplified")) {
            // The Overworld surface (depth 0) under another world preset's noise settings.
            NoiseGeneratorSettings ps = registries.lookupOrThrow(Registries.NOISE_SETTINGS)
                .getOrThrow(a[2].equals("amplified") ? NoiseGeneratorSettings.AMPLIFIED : NoiseGeneratorSettings.LARGE_BIOMES).value();
            Climate.Sampler ss = RandomState.create(registries.lookupOrThrow(Registries.NOISE), seed, ps).createClimateSampler(SamplerContext.EMPTY_UNCACHED);
            MultiNoiseBiomeSource os = MultiNoiseBiomeSource.createFromPreset(registries.lookupOrThrow(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST).getOrThrow(MultiNoiseBiomeSourceParameterLists.OVERWORLD));
            try (PrintStream f = new PrintStream(new BufferedOutputStream(new FileOutputStream(a[8])), false, StandardCharsets.UTF_8)) {
                for (int j = 0; j < h; j++) {
                    for (int i = 0; i < w; i++) {
                        int x = x0 + i * step, z = z0 + j * step;
                        Climate.TargetPoint t = ss.sample(x >> 2, 64 >> 2, z >> 2);
                        t = new Climate.TargetPoint(t.temperature(), t.humidity(), t.continentalness(), t.erosion(), 0L, t.weirdness());
                        f.println(os.getNoiseBiome(t).unwrapKey().map(k -> k.identifier().toString()).orElse("?"));
                    }
                }
            }
            out.println("wrote " + (w * h) + " cells to " + a[8]);
            return;
        }
        NoiseGeneratorSettings settings = registries.lookupOrThrow(Registries.NOISE_SETTINGS)
            .getOrThrow(nether ? NoiseGeneratorSettings.NETHER : NoiseGeneratorSettings.END).value();
        RandomState random = RandomState.create(registries.lookupOrThrow(Registries.NOISE), seed, settings);
        Climate.Sampler sampler = random.createClimateSampler(SamplerContext.EMPTY_UNCACHED);
        net.minecraft.world.level.biome.BiomeSource source = nether
            ? MultiNoiseBiomeSource.createFromPreset(registries.lookupOrThrow(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST).getOrThrow(MultiNoiseBiomeSourceParameterLists.NETHER))
            : net.minecraft.world.level.biome.TheEndBiomeSource.create(registries.lookupOrThrow(Registries.BIOME));
        net.minecraft.world.level.biome.BiomeResolver resolver = source.createResolver(sampler);
        try (PrintStream f = new PrintStream(new BufferedOutputStream(new FileOutputStream(a[8])), false, StandardCharsets.UTF_8)) {
            for (int j = 0; j < h; j++) {
                for (int i = 0; i < w; i++) {
                    int x = x0 + i * step, z = z0 + j * step;
                    Holder<Biome> b = resolver.getNoiseBiome(x >> 2, 64 >> 2, z >> 2);
                    f.println(b.unwrapKey().map(k -> k.identifier().toString()).orElse("?"));
                }
            }
        }
        out.println("wrote " + (w * h) + " cells to " + a[8]);
    }

    private static JsonArray range(Climate.Parameter p) {
        JsonArray a = new JsonArray();
        a.add(p.min());
        a.add(p.max());
        return a;
    }
}
