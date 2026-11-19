package org.flare.hello;

import android.app.Activity;
import android.os.Build;
import android.os.Bundle;
import android.util.Log;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/** A plain Android app, built with javac + d8 + aapt2 against android.jar.
 *  On Flare it runs without any of Android's userspace. */
public class MainActivity extends Activity {
    private static final String TAG = "FlareHello";
    private static int created;

    interface Shape { double area(); String name(); }

    static final class Rect implements Shape {
        private final double w, h;
        Rect(double w, double h) { this.w = w; this.h = h; }
        public double area() { return w * h; }
        public String name() { return "rect"; }
    }

    static final class Circle implements Shape {
        private final double r;
        Circle(double r) { this.r = r; }
        public double area() { return Math.PI * r * r; }
        public String name() { return "circle"; }
    }

    static long fib(int n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        created++;
        Log.i(TAG, "onCreate on " + Build.MANUFACTURER + " " + Build.MODEL + ", API " + Build.VERSION.SDK_INT);

        int sum = 0;
        for (int i = 1; i <= 100; i++) sum += i;
        Log.i(TAG, "sum 1..100 = " + sum + ", fib(20) = " + fib(20) + ", 7/2.0 = " + (7 / 2.0));

        List<Shape> shapes = new ArrayList<>();
        shapes.add(new Rect(3, 4));
        shapes.add(new Circle(1));
        double total = 0;
        StringBuilder names = new StringBuilder();
        for (Shape s : shapes) {
            total += s.area();
            if (names.length() > 0) names.append(',');
            names.append(s.name());
        }
        Log.i(TAG, "shapes " + names + " total area " + String.format("%.3f", total));

        Map<String, Integer> counts = new HashMap<>();
        for (String w : "the quick brown fox jumps over the lazy dog the end".split(" ")) {
            Integer c = counts.get(w);
            counts.put(w, c == null ? 1 : c + 1);
        }
        Log.i(TAG, "word 'the' appears " + counts.get("the") + " times, " + counts.size() + " distinct words");

        int[] squares = new int[10];
        for (int i = 0; i < squares.length; i++) squares[i] = i * i;
        Log.i(TAG, "squares[9] = " + squares[9]);

        try {
            Object o = shapes.get(5);
            Log.e(TAG, "no exception?! " + o);
        } catch (IndexOutOfBoundsException e) {
            Log.i(TAG, "caught " + e.getClass().getSimpleName());
        }
        try {
            throw new IllegalStateException("custom failure");
        } catch (RuntimeException e) {
            Log.i(TAG, "caught " + e.getMessage());
        }
        Runnable r = () -> Log.i(TAG, "lambda ran, created = " + created);
        r.run();
    }

    @Override
    protected void onResume() {
        super.onResume();
        Log.i(TAG, "onResume: app is running");
    }
}
