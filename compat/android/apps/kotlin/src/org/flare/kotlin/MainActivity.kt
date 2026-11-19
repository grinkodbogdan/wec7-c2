package org.flare.kotlin

import android.app.Activity
import android.os.Build
import android.os.Bundle
import android.util.Log

private const val TAG = "FlareKotlin"

data class Task(val id: Int, val title: String, val done: Boolean = false)

sealed class Shape {
    data class Circle(val r: Double) : Shape()
    data class Square(val side: Double) : Shape()
    object Empty : Shape()
}

fun Shape.area(): Double = when (this) {
    is Shape.Circle -> Math.PI * r * r
    is Shape.Square -> side * side
    Shape.Empty -> 0.0
}

fun String.shout() = uppercase() + "!"

class MainActivity : Activity() {
    private var greeting: String? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Log.i(TAG, "Kotlin onCreate on ${Build.MANUFACTURER} (API ${Build.VERSION.SDK_INT})")

        val tasks = mutableListOf(Task(1, "boot kernel", true), Task(2, "run apk"), Task(3, "draw ui"))
        tasks += Task(4, "ship it")
        val open = tasks.filter { !it.done }.map { it.title }
        Log.i(TAG, "open tasks: ${open.joinToString(", ")}")
        val first = tasks.first()
        Log.i(TAG, "data class: $first, copy: ${first.copy(done = false)}, equal: ${first == Task(1, "boot kernel", true)}")

        val shapes = listOf(Shape.Circle(1.0), Shape.Square(2.0), Shape.Empty)
        val total = shapes.sumOf { it.area() }
        Log.i(TAG, "sealed when: total area %.2f".format(total))

        val counts = "a b a c b a".split(" ").groupingBy { it }.eachCount()
        Log.i(TAG, "groupingBy: $counts")

        greeting = greeting ?: "hello from kotlin".shout()
        Log.i(TAG, "elvis + extension: ${greeting?.length} chars: $greeting")

        val squares = (1..5).map { it * it }
        Log.i(TAG, "range map: $squares, sum ${squares.sum()}, max ${squares.maxOrNull()}")

        val result = runCatching { require(tasks.size > 10) { "only ${tasks.size} tasks" } }
        Log.i(TAG, "runCatching: ${result.exceptionOrNull()?.message}")
    }

    override fun onResume() {
        super.onResume()
        Log.i(TAG, "onResume: kotlin app is running")
    }
}
