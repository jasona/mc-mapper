package mcmap.oracle;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.FileOutputStream;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeSet;
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.util.RandomSource;
import net.minecraft.util.datafix.DataFixers;
import net.minecraft.world.level.block.Mirror;
import net.minecraft.world.level.block.Rotation;
import net.minecraft.world.level.levelgen.structure.BoundingBox;
import net.minecraft.world.level.levelgen.structure.Structure;
import net.minecraft.world.level.levelgen.structure.pools.StructurePoolElement;
import net.minecraft.world.level.levelgen.structure.pools.StructureTemplatePool;
import net.minecraft.world.level.levelgen.structure.structures.JigsawStructure;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplate;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplateManager;
import net.minecraft.world.level.storage.LevelStorageSource;

/**
 * Template geometry the structure engine needs, which lives in NBT files rather than JSON:
 *   pools: for every start pool of a jigsaw structure, the pool's weighted list in the game's
 *          order (indexes into elements), and per element its bounding box at the origin for each
 *          rotation, its ground level delta and the positions of named jigsaws (the start anchors).
 *   templates: the ruined portal templates' bounding boxes for each rotation and mirror, with the
 *          pivot the structure uses (half the size, rounded down).
 */
final class Templates {
    static void run(String[] a) throws Exception {
        var tmp = Files.createTempDirectory("mcmap-oracle");
        LevelStorageSource.LevelStorageAccess storage = LevelStorageSource.createDefault(tmp).createAccess("w");
        MultiPackResourceManager rm = new MultiPackResourceManager(PackType.SERVER_DATA, List.of(ServerPacksSource.createVanillaPackSource().fullResources()));
        var layers = net.minecraft.server.RegistryLayer.createRegistryAccess();
        var pending = net.minecraft.tags.TagLoader.loadTagsForExistingRegistries(rm, layers.getLayer(net.minecraft.server.RegistryLayer.STATIC));
        var ctx = net.minecraft.tags.TagLoader.buildUpdatedLookups(layers.getAccessForLoading(net.minecraft.server.RegistryLayer.WORLD), pending);
        RegistryAccess.Frozen worldgen = net.minecraft.resources.RegistryDataLoader.load(rm, ctx, net.minecraft.resources.RegistryDataLoader.WORLD_REGISTRIES, Runnable::run).join();
        pending.forEach(net.minecraft.core.Registry.PendingTags::apply);
        RegistryAccess reg = layers.replaceFrom(net.minecraft.server.RegistryLayer.WORLD, worldgen).compositeAccess();
        StructureTemplateManager templates = new StructureTemplateManager(rm, storage, DataFixers.getDataFixer(), reg.lookupOrThrow(Registries.BLOCK));

        // Start pools and anchor names of every jigsaw structure.
        Set<Identifier> pools = new TreeSet<>();
        Set<Identifier> anchors = new TreeSet<>();
        for (Structure s : reg.lookupOrThrow(Registries.STRUCTURE)) {
            if (s instanceof JigsawStructure j) {
                var f = JigsawStructure.class.getDeclaredField("startPool"); f.setAccessible(true);
                var pool = (net.minecraft.core.Holder<?>) f.get(j);
                pool.unwrapKey().ifPresent(k -> pools.add(((net.minecraft.resources.ResourceKey<?>) k).identifier()));
                var n = JigsawStructure.class.getDeclaredField("startJigsawName"); n.setAccessible(true);
                ((java.util.Optional<?>) n.get(j)).ifPresent(id -> anchors.add((Identifier) id));
            }
        }
        JsonObject root = new JsonObject();
        JsonObject poolsOut = new JsonObject();
        var poolReg = reg.lookupOrThrow(Registries.TEMPLATE_POOL);
        var templatesField = StructureTemplatePool.class.getDeclaredField("templates");
        templatesField.setAccessible(true);
        for (Identifier id : pools) {
            StructureTemplatePool pool = poolReg.getValue(id);
            if (pool == null) continue;
            @SuppressWarnings("unchecked")
            List<StructurePoolElement> list = (List<StructurePoolElement>) templatesField.get(pool);
            Map<StructurePoolElement, Integer> index = new IdentityHashMap<>();
            List<StructurePoolElement> distinct = new ArrayList<>();
            JsonArray order = new JsonArray();
            for (StructurePoolElement e : list) {
                Integer i = index.get(e);
                if (i == null) { i = distinct.size(); distinct.add(e); index.put(e, i); }
                order.add(i);
            }
            JsonArray elements = new JsonArray();
            for (StructurePoolElement e : distinct) {
                JsonObject eo = new JsonObject();
                eo.addProperty("element", e.toString());
                eo.addProperty("ground_level_delta", e.getGroundLevelDelta());
                JsonArray boxes = new JsonArray();
                JsonObject named = new JsonObject();
                for (Rotation r : Rotation.values()) {
                    boxes.add(box(e.getBoundingBox(templates, BlockPos.ZERO, r)));
                    for (var jb : e.getShuffledJigsawBlocks(templates, BlockPos.ZERO, r, RandomSource.create(0))) {
                        if (!anchors.contains(jb.name())) continue;
                        String key = jb.name().toString();
                        if (!named.has(key)) {
                            JsonArray per = new JsonArray();
                            for (int k = 0; k < 4; k++) per.add(new JsonArray());
                            named.add(key, per);
                        }
                        JsonArray p = new JsonArray();
                        p.add(jb.pos().getX()); p.add(jb.pos().getY()); p.add(jb.pos().getZ());
                        named.getAsJsonArray(key).get(r.ordinal()).getAsJsonArray().add(p);
                    }
                }
                eo.add("boxes", boxes);
                eo.add("anchors", named);
                elements.add(eo);
            }
            JsonObject po = new JsonObject();
            po.add("order", order);
            po.add("elements", elements);
            poolsOut.add(id.toString(), po);
        }
        root.add("pools", poolsOut);

        JsonObject tOut = new JsonObject();
        List<String> portals = new ArrayList<>();
        for (int i = 1; i <= 10; i++) portals.add("ruined_portal/portal_" + i);
        for (int i = 1; i <= 3; i++) portals.add("ruined_portal/giant_portal_" + i);
        for (String p : portals) {
            StructureTemplate t = templates.getOrCreate(Identifier.withDefaultNamespace(p));
            var size = t.getSize();
            BlockPos pivot = new BlockPos(size.getX() / 2, 0, size.getZ() / 2);
            JsonObject to = new JsonObject();
            JsonArray s = new JsonArray(); s.add(size.getX()); s.add(size.getY()); s.add(size.getZ());
            to.add("size", s);
            JsonArray boxes = new JsonArray();
            for (Mirror m : new Mirror[] {Mirror.NONE, Mirror.FRONT_BACK})
                for (Rotation r : Rotation.values())
                    boxes.add(box(t.getBoundingBox(BlockPos.ZERO, r, pivot, m)));
            to.add("boxes", boxes);
            tOut.add("minecraft:" + p, to);
        }
        root.add("templates", tOut);
        root.addProperty("rotation_order", "none, clockwise_90, clockwise_180, counterclockwise_90; boxes for mirror none then front_back");
        try (PrintStream o = new PrintStream(new FileOutputStream(a[1]), false, StandardCharsets.UTF_8)) {
            o.println(root);
        }
        System.err.println("wrote " + poolsOut.size() + " pools and " + tOut.size() + " templates to " + a[1]);
    }

    private static JsonArray box(BoundingBox b) {
        JsonArray a = new JsonArray();
        a.add(b.minX()); a.add(b.minY()); a.add(b.minZ()); a.add(b.maxX()); a.add(b.maxY()); a.add(b.maxZ());
        return a;
    }
}
