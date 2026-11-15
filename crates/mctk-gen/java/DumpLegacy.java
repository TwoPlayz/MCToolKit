import java.lang.reflect.*;
import java.util.*;
public class DumpLegacy {
  public static void main(String[] a) throws Exception {
    String out = a[0];
    try (java.io.PrintWriter w = new java.io.PrintWriter(out, "UTF-8")) {
      Class<?> item = Class.forName("net.minecraft.util.datafix.fixes.ItemIdFix");
      Method gi = item.getMethod("getItem", int.class);
      for (int i = 0; i < 32768; i++) { String n = (String) gi.invoke(null, i); if (n != null && !(i > 0 && n.equals("minecraft:air"))) w.println("item\t" + i + "\t" + n); }
      Class<?> bs = Class.forName("net.minecraft.util.datafix.fixes.BlockStateData");
      Field f = bs.getDeclaredField("ID_BY_OLD_NAME"); f.setAccessible(true);
      Map<String,Integer> m = (Map<String,Integer>) f.get(null);
      for (Map.Entry<String,Integer> e : new TreeMap<>(m).entrySet()) w.println("block\t" + e.getKey() + "\t" + e.getValue());
      Class<?> en = Class.forName("net.minecraft.util.datafix.fixes.EntityIdFix");
      Method lm = en.getDeclaredMethod("lambda$static$0", HashMap.class); lm.setAccessible(true);
      HashMap<String,String> em = new HashMap<>(); lm.invoke(null, em);
      for (Map.Entry<String,String> e : new TreeMap<>(em).entrySet()) w.println("entity\t" + e.getKey() + "\t" + e.getValue());
    }
  }
}
