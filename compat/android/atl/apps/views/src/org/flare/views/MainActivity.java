package org.flare.views;

import android.app.Activity;
import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.RectF;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.view.Gravity;
import android.view.View;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.TextView;

/** A view-system check: layouts, framework widgets, a custom View with
 * onDraw, click handling and the main Looper. */
public class MainActivity extends Activity {
    static final String TAG = "FlareViews";
    int clicks;

    static class Dial extends View {
        final Paint p = new Paint(Paint.ANTI_ALIAS_FLAG);
        float angle = 30;

        Dial(Context c) {
            super(c);
        }

        @Override
        protected void onMeasure(int w, int h) {
            setMeasuredDimension(MeasureSpec.getSize(w), 260);
        }

        @Override
        protected void onDraw(Canvas c) {
            float cx = getWidth() / 2f, cy = getHeight() / 2f, r = Math.min(cx, cy) - 16;
            p.setStyle(Paint.Style.FILL);
            p.setColor(0xff1e88e5);
            c.drawCircle(cx, cy, r, p);
            p.setColor(0xffffc107);
            c.drawArc(new RectF(cx - r, cy - r, cx + r, cy + r), -90, angle, true, p);
            p.setColor(Color.WHITE);
            p.setStyle(Paint.Style.STROKE);
            p.setStrokeWidth(6);
            Path tri = new Path();
            tri.moveTo(cx - r * 0.4f, cy + r * 0.3f);
            tri.lineTo(cx, cy - r * 0.5f);
            tri.lineTo(cx + r * 0.4f, cy + r * 0.3f);
            tri.close();
            c.drawPath(tri, p);
            p.setStyle(Paint.Style.FILL);
            p.setTextSize(34);
            p.setTextAlign(Paint.Align.CENTER);
            c.drawText((int) angle + "°", cx, cy + r * 0.75f, p);
        }
    }

    @Override
    protected void onCreate(Bundle b) {
        super.onCreate(b);
        LinearLayout col = new LinearLayout(this);
        col.setOrientation(LinearLayout.VERTICAL);
        col.setPadding(24, 24, 24, 24);
        col.setBackgroundColor(0xfff4f6f8);

        TextView title = new TextView(this);
        title.setText("Android views on Flare");
        title.setTextSize(30);
        title.setTextColor(0xff102030);
        title.setGravity(Gravity.CENTER_HORIZONTAL);
        col.addView(title);

        final TextView status = new TextView(this);
        status.setText("ART + ATL, drawn natively into Arcan");
        status.setTextColor(0xff506070);
        status.setGravity(Gravity.CENTER_HORIZONTAL);
        status.setPadding(0, 8, 0, 16);
        col.addView(status);

        final Dial dial = new Dial(this);
        col.addView(dial);

        final EditText name = new EditText(this);
        name.setHint("type here");
        col.addView(name);

        LinearLayout row = new LinearLayout(this);
        row.setOrientation(LinearLayout.HORIZONTAL);
        final Button turn = new Button(this);
        turn.setText("Turn");
        Button reset = new Button(this);
        reset.setText("Reset");
        row.addView(turn, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1));
        row.addView(reset, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1));
        col.addView(row);

        turn.setOnClickListener(new View.OnClickListener() {
            public void onClick(View v) {
                clicks++;
                dial.angle = (dial.angle + 45) % 360;
                dial.invalidate();
                status.setText("clicked " + clicks + "x" + (name.getText().length() > 0 ? ", hello " + name.getText() : ""));
                Log.i(TAG, "click " + clicks + " angle " + (int) dial.angle);
            }
        });
        reset.setOnClickListener(new View.OnClickListener() {
            public void onClick(View v) {
                clicks = 0;
                dial.angle = 0;
                dial.invalidate();
                status.setText("reset");
                Log.i(TAG, "reset");
            }
        });
        setContentView(col);
        new Handler(Looper.getMainLooper()).postDelayed(new Runnable() {
            public void run() {
                Log.i(TAG, "looper ok: layout " + dial.getWidth() + "x" + dial.getHeight());
            }
        }, 200);
        Log.i(TAG, "onCreate done");
    }

    @Override
    protected void onResume() {
        super.onResume();
        Log.i(TAG, "onResume");
    }
}
