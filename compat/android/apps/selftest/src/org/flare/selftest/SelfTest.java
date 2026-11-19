package org.flare.selftest;

import android.app.Activity;
import android.os.Bundle;
import android.util.Log;
import java.util.*;

/** Exercises the language and library surface the droid runtime provides.
 *  Prints one line per failure and a summary. */
public class SelfTest extends Activity {
    private static final String TAG = "SelfTest";
    private int passed, failed;
    static final int[] PRIMES;
    static { PRIMES = new int[] {2, 3, 5, 7, 11, 13}; }

    private void check(String what, boolean ok) {
        if (ok) passed++; else { failed++; Log.e(TAG, "FAIL " + what); }
    }
    private void eq(String what, Object got, Object want) {
        check(what + " (want " + want + ", got " + got + ")", Objects.equals(want, got));
    }

    enum Color { RED, GREEN, BLUE; Color next() { return values()[(ordinal() + 1) % values().length]; } }

    interface Greeter { String name(); default String greet() { return "hello " + name(); } }

    static abstract class Animal implements Comparable<Animal> {
        final String name; final int legs;
        Animal(String name, int legs) { this.name = name; this.legs = legs; }
        abstract String sound();
        public String toString() { return name + "(" + legs + ")"; }
        public int compareTo(Animal o) { return name.compareTo(o.name); }
    }
    static class Dog extends Animal implements Greeter {
        Dog() { super("dog", 4); }
        String sound() { return "woof"; }
        public String name() { return name; }
    }
    static class Bird extends Animal {
        Bird() { super("bird", 2); }
        String sound() { return "tweet"; }
    }

    static class AppException extends Exception {
        final int code;
        AppException(String m, int code) { super(m); this.code = code; }
    }

    static class Point {
        final int x, y;
        Point(int x, int y) { this.x = x; this.y = y; }
        @Override public boolean equals(Object o) {
            return o instanceof Point && ((Point) o).x == x && ((Point) o).y == y;
        }
        @Override public int hashCode() { return 31 * x + y; }
    }

    static int classify(int n) {
        switch (n) { case 1: return 10; case 2: return 20; case 3: return 30; case 4: return 40; default: return -1; }
    }
    static int sparse(int n) {
        switch (n) { case -100: return 1; case 7: return 2; case 1000: return 3; default: return 0; }
    }
    static String word(String s) {
        switch (s) { case "one": return "1"; case "two": return "2"; default: return "?"; }
    }
    static void fail(int code) throws AppException { throw new AppException("code " + code, code); }

    private final Object lock = new Object();
    private int counter;

    @Override
    protected void onCreate(Bundle b) {
        super.onCreate(b);
        // Arithmetic
        eq("int overflow", -2147483648, Integer.MAX_VALUE + 1);
        eq("long mul", 600851475143L * 3, 1802554425429L);
        eq("int div", -7 / 2, -3);
        eq("int rem", -7 % 2, -1);
        eq("shift", -16 >>> 28, 15);
        eq("long shift", 1L << 40, 1099511627776L);
        eq("double", 0.1 + 0.2 > 0.3, true);
        eq("float to int", (int) 3.99f, 3);
        eq("double to long", (long) -2.5, -2L);
        eq("char math", (char) ('a' + 2), 'c');
        eq("byte cast", (byte) 200, (byte) -56);
        eq("Math.sqrt", Math.sqrt(144.0), 12.0);
        eq("Math.max", Math.max(3, 9), 9);
        boolean thrown = false;
        try { int z = 0; int q = 5 / z; } catch (ArithmeticException e) { thrown = true; }
        check("divide by zero throws", thrown);

        // Control flow
        eq("packed switch", classify(3), 30);
        eq("packed default", classify(9), -1);
        eq("sparse switch", sparse(1000), 3);
        eq("string switch", word("two"), "2");
        eq("static init", PRIMES[5], 13);
        int[][] grid = new int[3][4];
        grid[2][3] = 7;
        eq("2d array", grid[2][3] + grid.length * 10 + grid[0].length, 41);

        // Objects
        Color c = Color.BLUE;
        eq("enum next", c.next(), Color.RED);
        eq("enum name", Color.GREEN.name(), "GREEN");
        eq("enum valueOf", Color.valueOf("BLUE").ordinal(), 2);
        Dog d = new Dog();
        eq("default method", d.greet(), "hello dog");
        List<Animal> zoo = new ArrayList<>(Arrays.asList(new Dog(), new Bird()));
        Collections.sort(zoo);
        eq("comparable sort", zoo.toString(), "[bird(2), dog(4)]");
        zoo.sort((x, y) -> y.legs - x.legs);
        eq("comparator lambda", zoo.get(0).sound(), "woof");
        check("instanceof", zoo.get(1) instanceof Bird && !(zoo.get(1) instanceof Greeter));
        Map<Point, String> pts = new HashMap<>();
        pts.put(new Point(1, 2), "a");
        eq("custom equals as key", pts.get(new Point(1, 2)), "a");

        // Exceptions
        try { fail(42); } catch (AppException e) { eq("checked exception", e.code + e.getMessage(), "42code 42"); }
        StringBuilder trace = new StringBuilder();
        try { try { trace.append("t"); throw new IllegalArgumentException("x"); } finally { trace.append("f"); } }
        catch (RuntimeException e) { trace.append("c"); }
        eq("finally", trace.toString(), "tfc");
        try { Object o = "str"; Integer i = (Integer) o; check("bad cast", false); }
        catch (ClassCastException e) { check("class cast", true); }
        try { String s = null; s.length(); check("npe", false); }
        catch (NullPointerException e) { check("null pointer", true); }
        try { int[] a = new int[2]; a[2] = 1; } catch (ArrayIndexOutOfBoundsException e) { check("array bounds", true); }

        // Strings and collections
        eq("format", String.format("%5d|%-4s|%04x|%.2f", 42, "ab", 255, Math.PI), "   42|ab  |00ff|3.14");
        eq("substring", "flare os".substring(6).toUpperCase(), "OS");
        eq("indexOf", "abcabc".indexOf("ca"), 2);
        eq("join", String.join("-", Arrays.asList("a", "b", "c")), "a-b-c");
        eq("char array", new String("xyz".toCharArray()), "xyz");
        eq("parseInt", Integer.parseInt("-123") + 1, -122);
        eq("Integer.toString", Integer.toString(255) + Long.toString(7L), "2557");
        eq("double toString", String.valueOf(1.0 / 4), "0.25");
        Set<String> set = new HashSet<>(Arrays.asList("x", "y", "x"));
        eq("set dedupe", set.size(), 2);
        Map<String, Integer> m = new LinkedHashMap<>();
        for (String w : "b a b c b".split(" ")) m.put(w, m.getOrDefault(w, 0) + 1);
        int total = 0;
        for (Map.Entry<String, Integer> e : m.entrySet()) total += e.getValue();
        eq("map iteration", total, 5);
        eq("map toString", m.toString(), "{b=3, a=1, c=1}");
        Iterator<Animal> it = zoo.iterator();
        it.next(); it.remove();
        eq("iterator remove", zoo.size(), 1);
        int[] arr = {5, 3, 9, 1};
        Arrays.sort(arr);
        eq("Arrays.sort", Arrays.toString(arr), "[1, 3, 5, 9]");
        int[] copy = new int[4];
        System.arraycopy(arr, 1, copy, 0, 3);
        eq("arraycopy", copy[2], 9);
        long sum = 0;
        for (int p : PRIMES) sum += p;
        eq("for-each array", sum, 41L);

        synchronized (lock) { counter++; }
        Runnable r = () -> counter += 10;
        r.run();
        eq("lambda capture this", counter, 11);
        Bundle bundle = new Bundle();
        bundle.putInt("n", 5);
        eq("Bundle", bundle.getInt("n"), 5);
        eq("package", getPackageName(), "org.flare.selftest");

        Log.i(TAG, "selftest: " + passed + "/" + (passed + failed) + " passed" + (failed == 0 ? "" : " (" + failed + " FAILED)"));
    }
}
