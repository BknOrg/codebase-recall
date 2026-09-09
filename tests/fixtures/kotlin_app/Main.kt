package app

import util.Greeter

fun main() {
    val g = Greeter()
    g.greet()
}

@Composable
fun Screen() {
    val g = Greeter()
    g.greet()
}
