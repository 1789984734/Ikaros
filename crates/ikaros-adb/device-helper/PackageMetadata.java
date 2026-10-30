package dev.ikaros;

import android.app.Application;
import android.app.Instrumentation;
import android.content.Context;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.drawable.Drawable;
import android.os.Build;
import android.os.Looper;
import android.os.UserHandle;
import android.util.Base64;
import java.io.ByteArrayOutputStream;
import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import org.json.JSONObject;

/** Read-only, short-lived ADB shell helper; no APK installation is required. */
public final class PackageMetadata {
    private static final int ICON_SIZE = 96;

    public static void main(String[] args) {
        try {
            PackageManager pm = createShellContext().getPackageManager();
            String filter = args.length == 0 ? "all" : args[0];
            int count = 0;
            for (PackageInfo pkg : pm.getInstalledPackages(0)) {
                ApplicationInfo app = pkg.applicationInfo;
                if (app == null || !matchesFilter(app, filter)) {
                    continue;
                }
                System.out.println("IKAROS_APP:" + describePackage(pm, pkg));
                count++;
            }
            System.out.println("IKAROS_DONE:" + count);
            System.exit(0);
        } catch (Throwable error) {
            System.out.println("IKAROS_ERROR:" + error);
            System.exit(1);
        }
    }

    private static boolean matchesFilter(ApplicationInfo app, String filter) {
        boolean system = (app.flags & ApplicationInfo.FLAG_SYSTEM) != 0;
        return "all".equals(filter) || ("system".equals(filter) == system);
    }

    private static JSONObject describePackage(PackageManager pm, PackageInfo pkg) throws Exception {
        ApplicationInfo app = pkg.applicationInfo;
        JSONObject result = new JSONObject();
        result.put("package_name", pkg.packageName);
        result.put("app_name", loadLabel(pm, app));
        result.put("version_name", pkg.versionName == null ? "" : pkg.versionName);
        long code = Build.VERSION.SDK_INT >= 28 ? pkg.getLongVersionCode() : pkg.versionCode;
        result.put("version_code", Long.toString(code));
        result.put("apk_path", app.sourceDir);
        result.put("is_system", (app.flags & ApplicationInfo.FLAG_SYSTEM) != 0);
        result.put("is_enabled", app.enabled);
        try {
            result.put("icon_png", renderIcon(app.loadIcon(pm)));
        } catch (RuntimeException error) {
            // A damaged resource in one package must not hide the other applications.
            result.put("icon_png", "");
        }
        return result;
    }

    private static String loadLabel(PackageManager pm, ApplicationInfo app) {
        try {
            CharSequence label = app.loadLabel(pm);
            return label == null ? app.packageName : label.toString();
        } catch (RuntimeException error) {
            return app.packageName;
        }
    }

    private static String renderIcon(Drawable drawable) {
        Bitmap bitmap = Bitmap.createBitmap(ICON_SIZE, ICON_SIZE, Bitmap.Config.ARGB_8888);
        try {
            drawable.setBounds(0, 0, ICON_SIZE, ICON_SIZE);
            drawable.draw(new Canvas(bitmap));
            ByteArrayOutputStream png = new ByteArrayOutputStream();
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, png);
            return Base64.encodeToString(png.toByteArray(), Base64.NO_WRAP);
        } finally {
            bitmap.recycle();
        }
    }

    private static Context createShellContext() throws Exception {
        Looper.prepareMainLooper();
        Class<?> threadType = Class.forName("android.app.ActivityThread");
        Object thread = construct(threadType);
        setField(threadType, null, "sCurrentActivityThread", thread);
        setField(threadType, thread, "mSystemThread", true);

        // Set the identity before creating resources: some OEM caches capture it once.
        ApplicationInfo shellInfo = new ApplicationInfo();
        shellInfo.packageName = "com.android.shell";
        shellInfo.processName = "com.android.shell";
        Class<?> bindType = Class.forName("android.app.ActivityThread$AppBindData");
        Object binding = construct(bindType);
        setField(bindType, binding, "appInfo", shellInfo);
        setField(bindType, binding, "processName", shellInfo.processName);
        setField(threadType, thread, "mBoundApplication", binding);

        if (Build.VERSION.SDK_INT >= 31) {
            Class<?> controllerType = Class.forName("android.app.ConfigurationController");
            Class<?> internalType = Class.forName("android.app.ActivityThreadInternal");
            Constructor<?> constructor = controllerType.getDeclaredConstructor(internalType);
            constructor.setAccessible(true);
            setField(threadType, thread, "mConfigurationController", constructor.newInstance(thread));
        }
        Context system = (Context) threadType.getMethod("getSystemContext").invoke(thread);
        Context context = createCurrentUserShellContext(system);
        Application app = Instrumentation.newApplication(Application.class, context);
        setField(threadType, thread, "mInitialApplication", app);
        return context;
    }

    private static Context createCurrentUserShellContext(Context system) throws Exception {
        int userId = (Integer) Class.forName("android.app.ActivityManager")
                .getMethod("getCurrentUser").invoke(null);
        Constructor<UserHandle> constructor = UserHandle.class.getDeclaredConstructor(int.class);
        constructor.setAccessible(true);
        UserHandle user = constructor.newInstance(userId);
        return (Context) Context.class
                .getMethod("createPackageContextAsUser", String.class, int.class, UserHandle.class)
                .invoke(system, "com.android.shell", 0, user);
    }

    private static Object construct(Class<?> type) throws Exception {
        Constructor<?> constructor = type.getDeclaredConstructor();
        constructor.setAccessible(true);
        return constructor.newInstance();
    }

    private static void setField(Class<?> type, Object target, String name, Object value) throws Exception {
        Field field = type.getDeclaredField(name);
        field.setAccessible(true);
        field.set(target, value);
    }
}
