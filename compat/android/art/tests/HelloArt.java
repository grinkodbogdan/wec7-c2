import java.util.Map;
import java.util.TreeMap;

/** Runs on real ART (Android's runtime) on Flare; checked by the boot test. */
public class HelloArt {
    static long fib(int n) {
        return n < 2 ? n : fib(n - 1) + fib(n - 2);
    }

    public static void main(String[] args) throws Exception {
        System.out.println("art: Hello from ART on Flare: " + System.getProperty("java.vm.name") + " "
                + System.getProperty("java.vm.version"));
        Map<String, Integer> m = new TreeMap<>();
        for (String w : "the quick brown fox jumps over the lazy dog the end".split(" ")) m.merge(w, 1, Integer::sum);
        System.out.println("art: words " + m);
        System.out.println("art: fib(25) = " + fib(25));
        Thread t = new Thread(() -> System.out.println("art: hello from " + Thread.currentThread().getName()));
        t.start();
        t.join();
        System.out.println(String.format("art: %d processors, pi = %.5f", Runtime.getRuntime().availableProcessors(), Math.PI));
        System.exit(0);
    }
}
